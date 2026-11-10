//! Commands a filter plug-in calls with `vc_call` ([`vectorcraft_plugins::host`]): which commands a
//! plug-in may never call ([`denied`]), the call itself ([`call`]), and the run around the calls
//! ([`transaction`]).
//!
//! A filter whose manifest's `commandAllowlist` names commands runs on the session's thread. Each
//! call it makes runs that command on the document the run is for, through [`Session::execute`] with
//! its params checked as any request's are, and sees the selection the calls before it left. The
//! calls and the filter's output are one undo step, named after the plug-in or its command; a run
//! that fails, traps, runs out of budget or is cancelled, or makes a call that fails, leaves every
//! document, its selection and its history as they were.
//!
//! A plug-in never calls a command that reads or writes a file or folder, uses the clipboard,
//! changes preferences, runs a script, an action or a plug-in (a plug-in command, or an effect a
//! plug-in adds), opens, closes or switches documents, or undoes or redoes, whatever its manifest
//! lists. A command whose params name a file or folder is refused unless [`NOT_FILES`] says that
//! key names part of the document in it. Every other command is refused too unless
//! [`reviewed::ALLOWED`] lists it, so a command added to the registry later is refused until
//! someone decides.

mod reviewed;

use std::sync::Arc;

use serde_json::Value;
use vectorcraft_plugins::host::{CALL_COMMAND_FAILED, CALL_DENIED_COMMAND, CALL_DISABLED, CALL_UNKNOWN_COMMAND, CallFailure};

use super::CommandSpec;
use crate::{EngineError, Result, Session};

/// Command ids a plug-in may never call, by their start: plug-ins, scripts and actions, files,
/// documents and links, the clipboard, preferences, the view and the app, and the undo history.
const DENIED_PREFIXES: &[&str] = &[
    "plugin.",
    "script.",
    "action.",
    "actionSet.",
    "command.",
    "prefs.",
    "help.",
    "engine.",
    "file.",
    "document.",
    "addon.document.",
    "library.",
    "links.",
    "recovery.",
    "shortcutSet.",
    "clipboard.",
    "view.",
    "print.",
    "pdf.",
    "webExport.",
    "css.",
    "assets.",
    "importMatches.",
    "edit.undo",
    "edit.redo",
    "edit.clearHistory",
    "edit.copy",
    "edit.cut",
    "edit.paste",
    "edit.newDocument",
    "edit.colorSettings",
    "edit.dropSelection",
    "artboard.copy",
    "artboard.cut",
    "text.rescanFonts",
    "text.fontList",
    "text.spelling.",
    "colorTheme.",
    "magicWand.",
    "eyedropper.",
    "layer.pasteRemembersLayers",
    "type.systemLayout",
    "type.fractionalWidths",
    "appearance.setNewArtBasic",
    "appearance.newArt",
    "perspective.widget",
    "transparency.toggleNewMasks",
];

/// Words in a command id that name a file or the clipboard (any case): a command with one of them
/// in its id may read or write a file a plug-in can't see.
const FILE_WORDS: &[&str] = &[
    "export",
    "import",
    "save",
    "load",
    "open",
    "file",
    "folder",
    "link",
    "librar",
    "preset",
    "recover",
    "package",
    "print",
    "profile",
    "clipboard",
];

/// Param keys that name a file or a folder.
const FILE_KEYS: &[&str] = &["path", "paths", "file", "files", "folder", "folders", "dir", "directory", "destinationFolder", "sourceFolder", "url"];

/// Commands and the param key in [`FILE_KEYS`] that, in them, names part of the document (a path of
/// its art, a setting, an attribute it stores or a result field), not a file.
pub(crate) const NOT_FILES: &[(&str, &str)] = &[
    ("object.setLiveShape", "path"),
    ("path.reverse", "paths"),
    ("object.pathfinder.options", "paths"),
    ("object.maskImage", "path"),
    ("text.createInPath", "path"),
    ("imageTrace.make", "paths"),
    ("symbol.instance.setAppearance", "path"),
    ("symbol.instance.selectPart", "path"),
    ("object.objectsOnPath.attach", "path"),
    ("attributes.set", "url"),
    ("svgInteractivity.info", "files"),
    ("addon.cleanup.rejoin", "paths"),
];

/// Is command id `id` one a plug-in may never call, from its id alone ([`DENIED_PREFIXES`],
/// [`FILE_WORDS`])?
fn denied_id(id: &str) -> bool {
    let lower = id.to_ascii_lowercase();
    DENIED_PREFIXES.iter().any(|p| id.starts_with(p)) || FILE_WORDS.iter().any(|w| lower.contains(w))
}

/// Why a plug-in may never call `spec`, or `None` when it may (when its manifest lists it): its id
/// or a file key ([`denied_by_rule`]), [`reviewed::DENIED`], or no place in [`reviewed::ALLOWED`].
pub(crate) fn denied(spec: &CommandSpec) -> Option<&'static str> {
    if let Some(why) = denied_by_rule(spec) {
        return Some(why);
    }
    if reviewed::DENIED.binary_search(&spec.id).is_ok() {
        return Some("reviewed");
    }
    if reviewed::ALLOWED.binary_search(&spec.id).is_err() {
        return Some("not reviewed");
    }
    None
}

/// Why a plug-in may never call `spec` from its id ([`denied_id`]) or its params (a key in
/// [`FILE_KEYS`] that [`NOT_FILES`] doesn't name for it), or `None`.
fn denied_by_rule(spec: &CommandSpec) -> Option<&'static str> {
    let id = spec.id;
    if denied_id(id) {
        return Some("id");
    }
    let mut keys = FILE_KEYS.iter().filter(|k| !NOT_FILES.contains(&(id, **k)));
    let in_schema = |k: &str| spec.schema.is_some_and(|s| s.params.iter().any(|p| p.name == k));
    if keys.any(|k| names_key(spec.params, k) || in_schema(k)) {
        return Some("file key");
    }
    None
}

/// The ids `m`'s `commandAllowlist` names that no command has (a later version may add them; a
/// call of one fails), and those a plug-in may never call ([`denied`]).
pub(crate) fn check_allowlist(m: &vectorcraft_plugins::Manifest) -> (Vec<&str>, Vec<&str>) {
    let (mut unknown, mut refused) = (Vec::new(), Vec::new());
    for id in &m.command_allowlist {
        match super::find_command(id) {
            Some(spec) if denied(spec).is_some() => refused.push(id.as_str()),
            Some(_) => {}
            None if denied_id(id) => refused.push(id.as_str()),
            None => unknown.push(id.as_str()),
        }
    }
    (unknown, refused)
}

/// Does params text `text` name the key `key`: `key:` or `key?:`, not as the end of a longer word?
pub(crate) fn names_key(text: &str, key: &str) -> bool {
    text.match_indices(key).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = &text[at + key.len()..];
        before.is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '.')) && (after.starts_with(':') || after.starts_with("?:"))
    })
}

/// Param keys whose value names an effect or a command (`id` is `effect.apply`'s other name for
/// `effect`).
const PLUGIN_KEYS: &[&str] = &["effect", "effects", "id", "command", "commands"];

/// Does `v` name a plug-in anywhere: an effect or a command ([`PLUGIN_KEYS`]) whose id starts with
/// `plugin.`, at any depth? A call that does is refused, so a plug-in never runs another, or
/// itself. Other strings, such as text, may start with `plugin.`.
fn names_plugin(v: &Value) -> bool {
    names_plugin_in(v, false)
}

/// [`names_plugin`] for `v`, the value of a key in [`PLUGIN_KEYS`] (or an item of one) when
/// `named`.
fn names_plugin_in(v: &Value, named: bool) -> bool {
    match v {
        Value::String(s) => named && s.starts_with(vectorcraft_plugins::PREFIX),
        Value::Array(a) => a.iter().any(|x| names_plugin_in(x, named)),
        Value::Object(o) => o.iter().any(|(k, x)| names_plugin_in(x, PLUGIN_KEYS.contains(&k.as_str()))),
        _ => false,
    }
}

/// Commits an edit in progress in the active document (not a preview's), as [`transaction`] does →
/// whether that changed the document or its selection. A run that calls commands is prepared again
/// then, so its input is the document its calls change.
pub(crate) fn settle(s: &mut Session) -> Result<bool> {
    if s.previewing {
        return Ok(false);
    }
    let Some(before) = s.active().map(|d| (d.doc.clone(), d.selection.clone())) else { return Ok(false) };
    super::edit::settle_interaction(s)?;
    Ok(s.active().is_none_or(|d| !Arc::ptr_eq(&before.0, &d.doc) || before.1 != d.selection))
}

/// What a run that makes command calls keeps fixed: the document it runs in, the documents open,
/// whether an interaction holds its changes (its own, or a preview's), and whether the plug-in may
/// read and change the document.
pub(crate) struct Calls {
    document: Option<u64>,
    open: Vec<u64>,
    interaction: bool,
    may_read: bool,
    may_write: bool,
}

impl Calls {
    /// The run's fixed state in `s` as it is now; `may_read` and `may_write` are whether it has
    /// `doc.read` and `doc.write`.
    pub(crate) fn of(s: &Session, may_read: bool, may_write: bool) -> Calls {
        let (document, open) = (s.active().map(|d| d.uid), s.docs.iter().map(|d| d.uid).collect());
        Calls { document, open, interaction: s.in_interaction(), may_read, may_write }
    }

    /// Is `s` still as the run needs it: the same documents open, the same one active and the
    /// interaction holding the run's changes still open?
    fn kept(&self, s: &Session) -> bool {
        s.active().map(|d| d.uid) == self.document
            && s.docs.iter().map(|d| d.uid).eq(self.open.iter().copied())
            && (!self.interaction || s.in_interaction())
    }
}

/// Makes plug-in call `id` with `args` in `s` → its result, or why it didn't run or failed. A
/// command a plug-in may never call ([`denied`]), one whose args name a plug-in, one that changes
/// which documents are open or active or ends the run's edit, and one that changes the document for
/// a plug-in without `doc.write` are refused, and so is every call of a plug-in without `doc.read`:
/// a command's result describes the document.
pub(crate) fn call(s: &mut Session, run: &Calls, id: &str, args: &Value) -> std::result::Result<Value, CallFailure> {
    let refused = || CallFailure { code: CALL_DENIED_COMMAND, why: EngineError::Other("a plug-in may not call this command".into()).to_string() };
    if !run.may_read {
        return Err(refused());
    }
    let Some(spec) = super::find_command(id) else {
        return Err(CallFailure { code: CALL_UNKNOWN_COMMAND, why: super::params::unknown_command(id).to_string() });
    };
    if denied(spec).is_some() || names_plugin(args) {
        return Err(refused());
    }
    let before = s.active().map(|d| d.doc.clone());
    let r = s.execute(id, args);
    let value = match r {
        Ok(v) => v,
        Err(e @ EngineError::Disabled(..)) => return Err(CallFailure { code: CALL_DISABLED, why: e.to_string() }),
        Err(e) => return Err(CallFailure { code: CALL_COMMAND_FAILED, why: e.to_string() }),
    };
    let changed = before.is_some_and(|b| s.active().is_some_and(|d| !Arc::ptr_eq(&b, &d.doc)));
    if !run.kept(s) || (changed && !run.may_write) {
        return Err(refused());
    }
    Ok(value)
}

/// Runs `f`, a plug-in run that makes command calls, as one undo step named `label` in the active
/// document: the commands it calls and the output it applies change the document inside one
/// interaction, which is kept when `f` succeeds. When `f` fails, every document goes back to how it
/// was (its art, selection, interaction and history), and so does the session state the commands a
/// plug-in may call change ([`Saved`]). During a preview the preview's own interaction holds the
/// changes.
pub(crate) fn transaction(s: &mut Session, label: &str, f: impl FnOnce(&mut Session) -> Result<Value>) -> Result<Value> {
    if s.active().is_some() && !s.previewing {
        super::edit::settle_interaction(s)?;
    }
    let own = s.active().is_some() && !s.previewing && !s.in_interaction();
    if own {
        s.begin_interaction(label)?;
    }
    let saved = Saved::of(s);
    let start = s.docs.clone();
    let active = s.active;
    match f(s) {
        Ok(v) => {
            if own
                && let Some(st) = s.active_mut()
                && let Some(it) = st.interaction.take()
            {
                st.keep_interaction(it);
            }
            Ok(v)
        }
        Err(e) => {
            let open = std::mem::replace(&mut s.docs, start);
            for st in &mut s.docs {
                // Past the newest revision the document reached, so the restored one redraws.
                let newest = open.iter().filter(|d| d.uid == st.uid).map(|d| d.revision).max().unwrap_or(st.revision);
                st.revision = newest.max(st.revision).saturating_add(1);
                crate::deselect_hidden(&s.menu, &st.doc, &mut st.selection);
            }
            s.active = active;
            if own && let Some(st) = s.active_mut() {
                st.interaction = None;
            }
            saved.restore(s);
            Err(e)
        }
    }
}

/// The session state outside the documents that a command a plug-in may call changes: the paint
/// defaults and the colors it remembers, the drawing mode, the clipboard, the undo group, the
/// preferences and menu checks, the selected gradient stop, mesh point and Appearance item, and the
/// options a command keeps for its next run. [`transaction`] puts it back when a run fails.
struct Saved {
    paint: crate::PaintDefaults,
    fill_active: bool,
    draw_mode: crate::DrawMode,
    draw_inside: Option<vectorcraft_doc::NodeId>,
    clipboard: crate::Clipboard,
    untitled_counter: u32,
    explicit_undo_group: Option<Option<String>>,
    prefs: crate::Prefs,
    menu: super::menucmds::MenuState,
    gradient_stop: Option<(usize, super::gradient::StopOwner)>,
    freeform_point: Option<(usize, super::gradient::StopOwner)>,
    active_appearance_item: Option<super::appearance::ActiveItem>,
    eyedropper_sample: Option<Arc<vectorcraft_doc::Node>>,
    last_solid: vectorcraft_color::Color,
    last_gradient: vectorcraft_color::GradientPaint,
    recent_colors: Vec<vectorcraft_color::Color>,
    recent_urls: Vec<String>,
    global_edit_options: super::globaledit::Options,
    envelope_defaults: Option<(vectorcraft_doc::live::EnvelopeOptions, f64)>,
}

impl Saved {
    fn of(s: &Session) -> Saved {
        Saved {
            paint: s.paint.clone(),
            fill_active: s.fill_active,
            draw_mode: s.draw_mode,
            draw_inside: s.draw_inside,
            clipboard: s.clipboard.clone(),
            untitled_counter: s.untitled_counter,
            explicit_undo_group: s.explicit_undo_group.clone(),
            prefs: s.prefs.clone(),
            menu: s.menu.clone(),
            gradient_stop: s.gradient_stop,
            freeform_point: s.freeform_point,
            active_appearance_item: s.active_appearance_item.clone(),
            eyedropper_sample: s.eyedropper_sample.clone(),
            last_solid: s.last_solid,
            last_gradient: s.last_gradient.clone(),
            recent_colors: s.recent_colors.clone(),
            recent_urls: s.recent_urls.clone(),
            global_edit_options: s.global_edit_options.clone(),
            envelope_defaults: s.envelope_defaults,
        }
    }

    fn restore(self, s: &mut Session) {
        s.paint = self.paint;
        s.fill_active = self.fill_active;
        s.draw_mode = self.draw_mode;
        s.draw_inside = self.draw_inside;
        s.clipboard = self.clipboard;
        s.untitled_counter = self.untitled_counter;
        s.explicit_undo_group = self.explicit_undo_group;
        s.prefs = self.prefs;
        s.menu = self.menu;
        s.gradient_stop = self.gradient_stop;
        s.freeform_point = self.freeform_point;
        s.active_appearance_item = self.active_appearance_item;
        s.eyedropper_sample = self.eyedropper_sample;
        s.last_solid = self.last_solid;
        s.last_gradient = self.last_gradient;
        s.recent_colors = self.recent_colors;
        s.recent_urls = self.recent_urls;
        s.global_edit_options = self.global_edit_options;
        s.envelope_defaults = self.envelope_defaults;
        s.pending_paint = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every command in the registry is sorted: the ones a plug-in may call name no file, folder,
    /// clipboard, preference or script in their params text, and the known file, document, script
    /// and history commands are refused.
    #[test]
    fn every_command_is_sorted() {
        let specs = crate::cmd::command_specs();
        for id in [
            "document.save",
            "file.revert",
            "document.serialize",
            "links.update",
            "graphicStyle.libraries",
            "symbol.libraries",
            "brush.libraries",
            "text.findFontFiles",
            "plugin.run",
            "plugin.install",
            "command.batch",
            "edit.undo",
            "edit.redo",
            "edit.copy",
            "edit.paste",
            "prefs.set",
        ] {
            let Some(spec) = crate::cmd::find_command(id) else { panic!("no command {id}") };
            assert!(denied(spec).is_some(), "{id} is allowed");
        }
        for id in ["select.all", "object.transform", "path.reverse", "symbol.instance.selectPart"] {
            let Some(spec) = crate::cmd::find_command(id) else { panic!("no command {id}") };
            assert_eq!(denied(spec), None, "{id} is refused");
        }
        for (id, key) in NOT_FILES {
            let Some(spec) = crate::cmd::find_command(id) else { panic!("no command {id}") };
            assert!(names_key(spec.params, key), "{id} has no `{key}` param");
        }
        let leaks: Vec<&str> = specs
            .iter()
            .filter(|c| denied(c).is_none())
            .filter(|c| {
                let lower = c.id.to_ascii_lowercase();
                FILE_WORDS.iter().any(|w| lower.contains(w))
                    || FILE_KEYS.iter().any(|k| names_key(c.params, k) && !NOT_FILES.contains(&(c.id, *k)))
                    || names_key(c.params, "sourceDocument")
            })
            .map(|c| c.id)
            .collect();
        assert!(leaks.is_empty(), "allowed commands that name a file or another document: {leaks:?}");
        let allowed = specs.iter().filter(|c| denied(c).is_none()).count();
        assert!(allowed > 0 && allowed < specs.len());
    }

    /// Every command no rule refuses is in exactly one of [`reviewed::ALLOWED`] and
    /// [`reviewed::DENIED`], and they name only such commands, in order: a command added to the
    /// registry fails here until someone puts it in one.
    #[test]
    fn every_command_is_reviewed() {
        for list in [reviewed::ALLOWED, reviewed::DENIED] {
            assert!(list.windows(2).all(|w| w[0] < w[1]), "a reviewed list is out of order or names a command twice");
        }
        let (allowed, refused) = (|id: &str| reviewed::ALLOWED.binary_search(&id).is_ok(), |id: &str| reviewed::DENIED.binary_search(&id).is_ok());
        let unreviewed: Vec<&str> =
            crate::cmd::command_specs().iter().filter(|c| denied_by_rule(c).is_none() && allowed(c.id) == refused(c.id)).map(|c| c.id).collect();
        assert!(unreviewed.is_empty(), "commands in neither or both of reviewed::ALLOWED and reviewed::DENIED: {unreviewed:?}");
        let stale: Vec<&str> = reviewed::ALLOWED
            .iter()
            .chain(reviewed::DENIED)
            .copied()
            .filter(|id| crate::cmd::find_command(id).is_none_or(|c| denied_by_rule(c).is_some()))
            .collect();
        assert!(stale.is_empty(), "reviewed commands that no command has or a rule refuses: {stale:?}");
    }

    /// Only an effect or a command naming a plug-in counts, at any depth; text that happens to
    /// start with `plugin.` doesn't.
    #[test]
    fn a_plugin_is_named_only_as_an_effect_or_a_command() {
        use serde_json::json;
        assert!(names_plugin(&json!({"effect": "plugin.test.fx"})));
        assert!(names_plugin(&json!({"id": "plugin.test.fx"})));
        assert!(names_plugin(&json!({"effects": ["builtin.blur", "plugin.test.fx"]})));
        assert!(names_plugin(&json!({"items": [{"effect": "plugin.test.fx", "params": {}}]})));
        assert!(names_plugin(&json!({"command": "plugin.test.run"})));
        assert!(!names_plugin(&json!({"text": "plugin.example"})));
        assert!(!names_plugin(&json!({"name": "plugin.style", "effect": "drop-shadow"})));
        assert!(!names_plugin(&json!({"effect": {"name": "plugin.style"}})));
    }

    #[test]
    fn keys_are_found_as_whole_keys() {
        assert!(names_key("{path: a .wasm file}", "path"));
        assert!(names_key("{ids?: [id], path?: string}", "path"));
        assert!(!names_key("{subpath: 1}", "path"));
        assert!(!names_key("{x} on the path: the art", "the"));
        assert!(!names_key("{pathData: 1}", "path"));
        assert!(names_key("path: x", "path"));
    }
}
