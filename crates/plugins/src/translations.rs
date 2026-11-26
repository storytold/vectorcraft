//! Text a plug-in gives in other languages: its manifest's `translations` (docs/plugins.md ›
//! Translations).
//!
//! `translations` maps language codes (`de`, `pt-br`, `zh-hant`, compared without regard to case)
//! to the text shown in that language: the plug-in's `name`, its commands' labels, its parameters'
//! labels, help and choice labels, and its group headings. Every entry is optional; text a language
//! leaves out comes from the language without its last part (`pt-br`, then `pt`) and then from the
//! manifest's own text. The text is shown only: command ids, parameter names, the values a choice
//! passes, `plugin.list` and the install approval keep the manifest's own text.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::dialog::{MAX_HELP, MAX_LABEL};
use crate::manifest::{Command, Manifest, ParamSpec};
use crate::permissions::{self, MAX_SHOWN_FIELD, MAX_SHOWN_NAME};
use crate::{Error, Result};

/// Most languages a manifest's `translations` gives text in.
pub const MAX_LANGUAGES: usize = 16;
/// Longest language code, in bytes.
pub const MAX_LANGUAGE_CODE: usize = 35;
/// Longest translated plug-in name, command label or choice label, in characters.
pub const MAX_NAME: usize = 64;

/// Named entries in the order the manifest gives them; each name at most once.
#[derive(Clone, Debug, PartialEq)]
pub struct Entries<T>(pub Vec<(String, T)>);

impl<T> Default for Entries<T> {
    fn default() -> Self {
        Entries(Vec::new())
    }
}

impl<T> Entries<T> {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The entry named `name`.
    pub fn get(&self, name: &str) -> Option<&T> {
        self.0.iter().find(|(k, _)| k == name).map(|(_, v)| v)
    }
}

impl<T: Serialize> Serialize for Entries<T> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut m = s.serialize_map(Some(self.0.len()))?;
        for (k, v) in &self.0 {
            m.serialize_entry(k, v)?;
        }
        m.end()
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Entries<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for V<T> {
            type Value = Entries<T>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> std::result::Result<Self::Value, A::Error> {
                let mut seen = HashSet::new();
                let mut out = Vec::new();
                while let Some(k) = map.next_key::<String>()? {
                    if !seen.insert(k.clone()) {
                        let shown = permissions::display_text(&k, MAX_SHOWN_FIELD);
                        return Err(serde::de::Error::custom(format!("`{shown}` is given twice")));
                    }
                    out.push((k, map.next_value()?));
                }
                Ok(Entries(out))
            }
        }
        d.deserialize_any(V(std::marker::PhantomData))
    }
}

/// A parameter's text in one language.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParamText {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// A choice's labels, by the value each option passes.
    #[serde(default, skip_serializing_if = "Entries::is_empty")]
    pub options: Entries<String>,
}

/// A command's text in one language.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandText {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Entries::is_empty")]
    pub params: Entries<ParamText>,
}

/// A plug-in's text in one language.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// By parameter name.
    #[serde(default, skip_serializing_if = "Entries::is_empty")]
    pub params: Entries<ParamText>,
    /// Group headings, by the `group` the parameters give.
    #[serde(default, skip_serializing_if = "Entries::is_empty")]
    pub groups: Entries<String>,
    /// By command id.
    #[serde(default, skip_serializing_if = "Entries::is_empty")]
    pub commands: Entries<CommandText>,
}

/// A manifest's `translations`, by language code.
pub type Translations = Entries<Catalog>;

/// Is `tag` a language code: a language of 2 or 3 letters, then parts of 2 to 8 letters and
/// digits, each after a `-`, at most [`MAX_LANGUAGE_CODE`] bytes in all (`de`, `pt-BR`,
/// `zh-Hant-TW`)?
pub fn valid_language_code(tag: &str) -> bool {
    let mut parts = tag.split('-');
    tag.len() <= MAX_LANGUAGE_CODE
        && parts.next().is_some_and(|l| (2..=3).contains(&l.len()) && l.bytes().all(|b| b.is_ascii_alphabetic()))
        && parts.all(|p| (2..=8).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// The codes text for language `lang` is looked up under, most specific first: `lang`, then
/// `lang` without its last part, down to the language alone (`zh-hant-tw`, `zh-hant`, `zh`).
fn lookup_codes(lang: &str) -> impl Iterator<Item = &str> {
    std::iter::successors(Some(lang), |l| l.rfind('-').map(|i| &l[..i]))
}

impl Translations {
    /// The first text `f` finds for language `lang` ([`lookup_codes`]).
    fn find<'a, T>(&'a self, lang: &str, f: impl Fn(&'a Catalog) -> Option<T>) -> Option<T> {
        lookup_codes(lang).find_map(|code| self.0.iter().filter(|(k, _)| k.eq_ignore_ascii_case(code)).find_map(|(_, c)| f(c)))
    }

    /// The plug-in's name these translations give for language `lang` (`None`: they give none).
    pub fn name_in(&self, lang: &str) -> Option<&str> {
        self.find(lang, |c| c.name.as_deref())
    }
}

/// Is `s` text a translation may give: 1 to `max` characters, not all spaces, without control
/// characters (`\n` too, unless `lines`) or characters that change the direction text is laid out
/// in or separate lines?
fn fits(s: &str, max: usize, lines: bool) -> bool {
    !s.trim().is_empty()
        && s.chars().count() <= max
        && s.chars().all(|c| (!c.is_control() || (lines && c == '\n')) && !permissions::is_bidi_or_separator(c))
}

/// `translations.<path>` as an error names it, each part escaped as prompts show text.
fn key(path: &[&str]) -> String {
    let parts: Vec<String> = path.iter().map(|p| permissions::display_text(p, MAX_SHOWN_FIELD)).collect();
    format!("`translations.{}`", parts.join("."))
}

/// Text `s` as a dialog shows it: each line with its control and bidirectional characters escaped
/// and cut at [`MAX_HELP`] characters.
fn shown_text(s: &str) -> String {
    let lines: Vec<String> = s.split('\n').map(|l| permissions::display_text(l, MAX_HELP)).collect();
    lines.join("\n")
}

/// Checks text `s` at `path` against `max` characters (`lines`: `\n` allowed).
fn check_text(s: &str, max: usize, lines: bool, path: &[&str]) -> Result<()> {
    if fits(s, max, lines) {
        return Ok(());
    }
    let at = key(path);
    Err(Error::Manifest(format!("{at} must be 1-{max} printable characters, without direction or line-separator characters")))
}

/// The error for the key at `path`, which names a parameter, choice value, group or command the
/// manifest doesn't declare, or translates a field the manifest doesn't give (`options` of a
/// parameter that isn't a choice, `help` of a parameter without one).
fn undeclared(path: &[&str]) -> Error {
    let at = key(path);
    Error::Manifest(format!("{at} names or translates something the manifest doesn't declare"))
}

/// Checks the text of parameters `texts` against the parameters `params` they name, at `path`.
fn check_params(texts: &Entries<ParamText>, params: &[(String, crate::Param)], path: &[&str]) -> Result<()> {
    for (k, t) in &texts.0 {
        let here = [path, &[k.as_str()]].concat();
        let Some((_, p)) = params.iter().find(|(name, _)| name == k) else {
            return Err(undeclared(&here));
        };
        if let Some(label) = &t.label {
            check_text(label, MAX_LABEL, false, &[here.as_slice(), &["label"]].concat())?;
        }
        if let Some(help) = &t.help {
            let at = [here.as_slice(), &["help"]].concat();
            if p.ui.help.is_none() {
                return Err(undeclared(&at));
            }
            check_text(help, MAX_HELP, true, &at)?;
        }
        if t.options.is_empty() {
            continue;
        }
        let ParamSpec::Choice { options, .. } = &p.spec else {
            return Err(undeclared(&[here.as_slice(), &["options"]].concat()));
        };
        for (value, label) in &t.options.0 {
            let at = [here.as_slice(), &["options", value.as_str()]].concat();
            if !options.contains(value) {
                return Err(undeclared(&at));
            }
            check_text(label, MAX_NAME, false, &at)?;
        }
    }
    Ok(())
}

impl Manifest {
    /// `translations` gives at most [`MAX_LANGUAGES`] languages, each by a distinct language code
    /// ([`valid_language_code`]; case doesn't tell codes apart), and names only what the manifest
    /// declares: its parameters, their choices' values, its groups and its commands. Each text
    /// follows the rules of the text it translates, without direction or line-separator
    /// characters, and `help` translates a `help` the parameter gives. The error names the key.
    pub(crate) fn validate_translations(&self) -> Result<()> {
        let langs = &self.translations.0;
        if langs.len() > MAX_LANGUAGES {
            return Err(Error::Manifest(format!("`translations` gives at most {MAX_LANGUAGES} languages")));
        }
        let groups: HashSet<&str> =
            self.params.iter().chain(self.commands.iter().flat_map(|c| c.params.iter())).filter_map(|(_, p)| p.ui.group.as_deref()).collect();
        for (i, (lang, c)) in langs.iter().enumerate() {
            let lang = lang.as_str();
            if !valid_language_code(lang) || langs.get(..i).is_some_and(|before| before.iter().any(|(b, _)| b.eq_ignore_ascii_case(lang))) {
                let at = key(&[lang]);
                return Err(Error::Manifest(format!("{at} is not a language code such as `de` or `pt-BR`, or repeats one given before it")));
            }
            if let Some(name) = &c.name {
                check_text(name, MAX_NAME, false, &[lang, "name"])?;
            }
            check_params(&c.params, &self.params, &[lang, "params"])?;
            for (g, heading) in &c.groups.0 {
                let g = g.as_str();
                if !groups.contains(g) {
                    return Err(undeclared(&[lang, "groups", g]));
                }
                check_text(heading, MAX_LABEL, false, &[lang, "groups", g])?;
            }
            for (id, t) in &c.commands.0 {
                let id = id.as_str();
                let Some(command) = self.command(id) else {
                    return Err(undeclared(&[lang, "commands", id]));
                };
                if let Some(label) = &t.label {
                    check_text(label, MAX_NAME, false, &[lang, "commands", id, "label"])?;
                }
                check_params(&t.params, &command.params, &[lang, "commands", id, "params"])?;
            }
        }
        // A dropdown tells its options apart by their labels: in each language, with the labels a
        // language leaves out taken from the language without its region, no two may be the same.
        for (lang, _) in langs {
            self.check_option_labels(lang, &[lang, "params"])?;
        }
        for c in self.commands.iter().filter(|c| c.params.iter().any(|(_, p)| matches!(p.spec, ParamSpec::Choice { .. }))) {
            let own = self.for_command(c);
            for (lang, _) in langs {
                own.check_option_labels(lang, &[lang, "commands", c.id.as_str(), "params"])?;
            }
        }
        Ok(())
    }

    /// Checks that no two options of a choice show the same label in language `lang`, naming the
    /// choice's `options` under `path`. Labels are compared as dialogs show them, with control and
    /// bidirectional characters escaped, so an option value holding U+202E and a translation
    /// giving the text `\u{202E}` count as the same label.
    fn check_option_labels(&self, lang: &str, path: &[&str]) -> Result<()> {
        for (k, p) in &self.params {
            let ParamSpec::Choice { options, .. } = &p.spec else { continue };
            let shown: Vec<String> = options.iter().map(|o| shown_text(self.option_label_in(k, o, lang))).collect();
            if shown.iter().enumerate().any(|(i, s)| shown.get(..i).is_some_and(|before| before.contains(s))) {
                let at = key(&[path, &[k.as_str(), "options"]].concat());
                return Err(Error::Manifest(format!("{at} gives two options the same label")));
            }
        }
        Ok(())
    }

    /// The translations command `c`'s manifest ([`Manifest::for_command`]) shows: in each language,
    /// `c`'s label as its name, `c`'s parameters' text and the group headings.
    pub(crate) fn command_translations(&self, c: &Command) -> Translations {
        let langs = self.translations.0.iter().map(|(lang, cat)| {
            let own = cat.commands.get(&c.id);
            let catalog = Catalog {
                name: own.and_then(|t| t.label.clone()),
                params: own.map(|t| t.params.clone()).unwrap_or_default(),
                groups: cat.groups.clone(),
                commands: Entries::default(),
            };
            (lang.clone(), catalog)
        });
        Entries(langs.collect())
    }

    /// The plug-in's name in language `lang` (an app language code such as `de` or `pt-br`): its
    /// translation, else the manifest's `name`.
    pub fn name_in(&self, lang: &str) -> &str {
        self.translations.name_in(lang).unwrap_or(&self.name)
    }

    /// [`Manifest::name_in`] as menus show it: trimmed, control and bidirectional characters
    /// escaped, at most [`MAX_SHOWN_NAME`] characters.
    pub fn shown_name(&self, lang: &str) -> String {
        permissions::display_text(self.name_in(lang).trim(), MAX_SHOWN_NAME)
    }

    /// Command `c`'s label in language `lang`, as menus show it ([`Command::shown_label`]).
    pub fn shown_command_label(&self, c: &Command, lang: &str) -> String {
        let label = self.translations.find(lang, |t| t.commands.get(&c.id).and_then(|t| t.label.as_deref())).unwrap_or(&c.label);
        permissions::display_text(label.trim(), MAX_SHOWN_NAME)
    }

    /// The label command `c`'s menu item shows in language `lang` ([`Command::menu_label`]):
    /// [`Manifest::shown_command_label`], and for a command placed in another menu, followed by
    /// [`Manifest::shown_name`] in parentheses.
    pub fn shown_command_menu_label(&self, c: &Command, lang: &str) -> String {
        let label = self.shown_command_label(c, lang);
        match c.place {
            None => label,
            Some(_) => format!("{label} ({})", self.shown_name(lang)),
        }
    }

    /// Parameter `k`'s label in language `lang`: its translation, else its `label` (`None`: it
    /// has neither).
    pub fn param_label_in(&self, k: &str, lang: &str) -> Option<&str> {
        let own = self.param(k).and_then(|p| p.ui.label.as_deref());
        self.translations.find(lang, |c| c.params.get(k).and_then(|t| t.label.as_deref())).or(own)
    }

    /// Parameter `k`'s help in language `lang`: its translation, else its `help`.
    pub fn param_help_in(&self, k: &str, lang: &str) -> Option<&str> {
        let own = self.param(k).and_then(|p| p.ui.help.as_deref());
        self.translations.find(lang, |c| c.params.get(k).and_then(|t| t.help.as_deref())).or(own)
    }

    /// The label of value `option` of choice `k` in language `lang`: its translation, else the
    /// value itself.
    pub fn option_label_in<'a>(&'a self, k: &str, option: &'a str, lang: &str) -> &'a str {
        self.translations.find(lang, |c| c.params.get(k).and_then(|t| t.options.get(option)).map(String::as_str)).unwrap_or(option)
    }

    /// The heading of group `group` in language `lang`: its translation, else `group`.
    pub fn group_in<'a>(&'a self, group: &'a str, lang: &str) -> &'a str {
        self.translations.find(lang, |c| c.groups.get(group).map(String::as_str)).unwrap_or(group)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    const TRANSLATED: &str = r#"{"id":"org.example.tidy","name":"Tidy","kind":"filter",
        "params":{"mode":{"type":"choice","options":["soft","hard"],"label":"Mode","group":"Shape","help":"How hard."},
                  "steps":{"type":"int","min":1,"max":9,"group":"Shape"}},
        "commands":[{"id":"grow","label":"Grow","params":{"by":{"type":"number","min":0,"max":10,"default":2,"label":"By","group":"Shape"}}},
                    {"id":"shrink","label":"Shrink"}],
        "translations":{
            "de":{"name":"Aufräumen","params":{"mode":{"label":"Modus","help":"Wie stark.","options":{"soft":"Weich"}},"steps":{"label":"Schritte"}},
                  "groups":{"Shape":"Form"},
                  "commands":{"grow":{"label":"Wachsen","params":{"by":{"label":"Um"}}}}},
            "pt":{"name":"Arrumar","commands":{"shrink":{"label":"Encolher"}}},
            "pt-BR":{"name":"Organizar"}}}"#;

    fn translated() -> Manifest {
        Manifest::parse(TRANSLATED.as_bytes()).unwrap()
    }

    /// A German UI shows the German text; any language without a catalog shows the manifest's own.
    #[test]
    fn a_language_shows_its_text_and_others_the_manifests_own() {
        let m = translated();
        assert_eq!(m.name_in("de"), "Aufräumen");
        assert_eq!(m.shown_name("de"), "Aufräumen");
        assert_eq!(m.name_in("de-at"), "Aufräumen", "a region falls back to its language");
        for other in ["en", "fr", "ja", "zh-hant"] {
            assert_eq!(m.name_in(other), "Tidy", "{other}");
            assert_eq!(m.param_label_in("mode", other), Some("Mode"), "{other}");
            assert_eq!(m.param_label_in("steps", other), None, "{other}");
            assert_eq!(m.option_label_in("mode", "soft", other), "soft", "{other}");
            assert_eq!(m.group_in("Shape", other), "Shape", "{other}");
        }
        assert_eq!(m.param_label_in("mode", "de"), Some("Modus"));
        assert_eq!(m.param_label_in("steps", "de"), Some("Schritte"), "a label for a parameter that gives none");
        assert_eq!(m.param_help_in("mode", "de"), Some("Wie stark."));
        assert_eq!((m.option_label_in("mode", "soft", "de"), m.option_label_in("mode", "hard", "de")), ("Weich", "hard"));
        assert_eq!(m.group_in("Shape", "de"), "Form");
        let grow = m.command("grow").unwrap();
        assert_eq!((m.shown_command_label(grow, "de"), m.shown_command_label(grow, "en")), ("Wachsen".into(), "Grow".into()));
        // A command's dialog shows its own text in that language.
        let run = m.for_command(grow);
        assert_eq!((run.name_in("de"), run.param_label_in("by", "de"), run.group_in("Shape", "de")), ("Wachsen", Some("Um"), "Form"));
        assert_eq!((run.name_in("en"), run.param_label_in("by", "en")), ("Grow", Some("By")));
        // The text is shown only: what runs and what plugin.list reads keep the manifest's own.
        assert_eq!((m.name.as_str(), grow.label.as_str(), grow.id.as_str()), ("Tidy", "Grow", "grow"));
        assert_eq!(Value::Object(m.resolve_params(&json!({"mode": "soft"})).unwrap())["mode"], json!("soft"));
    }

    /// `pt-br` text comes from `pt-br`, then `pt`, then the manifest, each piece on its own.
    #[test]
    fn a_region_falls_back_to_its_language_and_then_the_manifest() {
        let m = translated();
        assert_eq!(m.name_in("pt-br"), "Organizar", "codes match without regard to case");
        assert_eq!(m.name_in("pt"), "Arrumar");
        assert_eq!(m.name_in("pt-pt"), "Arrumar");
        let shrink = m.command("shrink").unwrap();
        assert_eq!(m.shown_command_label(shrink, "pt-br"), "Encolher", "from pt");
        let grow = m.command("grow").unwrap();
        assert_eq!(m.shown_command_label(grow, "pt-br"), "Grow", "from the manifest");
    }

    /// A command placed in another menu names its plug-in after its label, both in the language
    /// shown; one in its plug-in's submenu shows its label alone.
    #[test]
    fn a_placed_command_names_its_plugin_in_the_language_shown() {
        let m = Manifest::parse(
            r#"{"id":"org.example.ridge","name":"Ridge","api":2,"kind":"filter",
                "commands":[{"id":"low","label":"Low Area","menu":{"path":["Select"]}},{"id":"stay","label":"Stay"}],
                "translations":{"de":{"name":"Grat","commands":{"low":{"label":"Tiefe Fläche"},"stay":{"label":"Bleiben"}}}}}"#
                .as_bytes(),
        )
        .unwrap();
        let (low, stay) = (m.command("low").unwrap(), m.command("stay").unwrap());
        assert_eq!(m.shown_command_menu_label(low, "de"), "Tiefe Fläche (Grat)");
        assert_eq!(m.shown_command_menu_label(low, "fr"), "Low Area (Ridge)");
        assert_eq!(m.shown_command_menu_label(low, "en"), low.menu_label(&m.name), "the manifest's own text as before");
        assert_eq!((m.shown_command_menu_label(stay, "de"), m.shown_command_menu_label(stay, "en")), ("Bleiben".into(), "Stay".into()));
    }

    #[test]
    fn translations_round_trip_and_are_left_out_when_absent() {
        let m = translated();
        let written = serde_json::to_value(&m).unwrap();
        assert_eq!(written["translations"]["pt-BR"], json!({"name": "Organizar"}));
        assert_eq!(Manifest::parse(&serde_json::to_vec(&m).unwrap()).unwrap(), m);
        let plain = Manifest::parse(br#"{"id":"a","name":"x","kind":"filter"}"#).unwrap();
        assert!(serde_json::to_value(&plain).unwrap().get("translations").is_none());
        assert_eq!(plain.name_in("de"), "x");
    }

    /// A bad code, text a source field couldn't hold, text that changes direction, and a key the
    /// manifest doesn't declare are refused with an error naming the key.
    #[test]
    fn bad_translations_are_refused_naming_the_key() {
        let with = |translations: &str| {
            format!(
                r#"{{"id":"a","name":"x","kind":"filter","params":{{"mode":{{"type":"choice","options":["a","b"],"group":"G"}},
                    "n":{{"type":"number","min":0,"max":1,"default":0}}}},"commands":[{{"id":"c","label":"C"}}],"translations":{translations}}}"#
            )
        };
        let long = "x".repeat(MAX_NAME + 1);
        let long_label = format!(r#"{{"de":{{"params":{{"n":{{"label":"{}"}}}}}}}}"#, "x".repeat(MAX_LABEL + 1));
        let long_name = format!(r#"{{"de":{{"name":"{long}"}}}}"#);
        for (translations, says) in [
            (r#"{"german":{"name":"X"}}"#, "`translations.german` is not a language code"),
            (r#"{"d":{"name":"X"}}"#, "`translations.d` is not a language code"),
            (r#"{"de-":{"name":"X"}}"#, "`translations.de-` is not a language code"),
            (r#"{"de_DE":{"name":"X"}}"#, "`translations.de_DE` is not a language code"),
            (r#"{"x-klingon":{"name":"X"}}"#, "`translations.x-klingon` is not a language code"),
            (r#"{"":{"name":"X"}}"#, "`translations.` is not a language code"),
            (r#"{"de":{"name":"X"},"DE":{"name":"Y"}}"#, "`translations.DE` is not a language code such as `de` or `pt-BR`, or repeats one"),
            (r#"{"de":{"name":""}}"#, "`translations.de.name` must be 1-64"),
            (r#"{"de":{"name":"  "}}"#, "`translations.de.name` must be 1-64"),
            (long_name.as_str(), "`translations.de.name` must be 1-64"),
            (r#"{"de":{"name":"a\u0007b"}}"#, "`translations.de.name`"),
            (r#"{"de":{"commands":{"c":{"label":"a\nb"}}}}"#, "`translations.de.commands.c.label` must be"),
            (
                r#"{"de":{"commands":{"nope":{"label":"N"}}}}"#,
                "`translations.de.commands.nope` names or translates something the manifest doesn't declare",
            ),
            (r#"{"de":{"params":{"nope":{"label":"N"}}}}"#, "`translations.de.params.nope` names or translates"),
            (long_label.as_str(), "`translations.de.params.n.label` must be 1-64"),
            (r#"{"de":{"params":{"n":{"help":"H"}}}}"#, "`translations.de.params.n.help` names or translates"),
            (r#"{"de":{"params":{"n":{"options":{"a":"A"}}}}}"#, "`translations.de.params.n.options` names or translates"),
            (r#"{"de":{"params":{"mode":{"options":{"c":"C"}}}}}"#, "`translations.de.params.mode.options.c` names or translates"),
            (r#"{"de":{"params":{"mode":{"options":{"a":""}}}}}"#, "`translations.de.params.mode.options.a` must be"),
            (r#"{"de":{"params":{"mode":{"options":{"a":"b"}}}}}"#, "`translations.de.params.mode.options` gives two options the same label"),
            (r#"{"de":{"params":{"mode":{"options":{"a":"X","b":"X"}}}}}"#, "gives two options the same label"),
            (
                r#"{"pt":{"params":{"mode":{"options":{"a":"Leve"}}}},"pt-BR":{"params":{"mode":{"options":{"b":"Leve"}}}}}"#,
                "`translations.pt-BR.params.mode.options` gives two options the same label",
            ),
            (r#"{"de":{"groups":{"H":"Ha"}}}"#, "`translations.de.groups.H` names or translates"),
        ] {
            let e = Manifest::parse(with(translations).as_bytes()).unwrap_err().to_string();
            assert!(e.contains(says), "{translations}: {e}");
        }
        // Characters that change the direction of the text or separate lines, written as JSON
        // escapes: right-to-left and left-to-right overrides, isolates, and a line separator.
        let esc = |c: u32| format!("\\u{c:04X}");
        for (translations, says) in [
            (format!(r#"{{"de":{{"name":"Flip{}txt"}}}}"#, esc(0x202E)), "`translations.de.name` must be"),
            (format!(r#"{{"de":{{"name":"Flip{}txt"}}}}"#, esc(0x2067)), "`translations.de.name` must be"),
            (format!(r#"{{"de":{{"name":"a{}b"}}}}"#, esc(0x2028)), "`translations.de.name` must be"),
            (format!(r#"{{"de":{{"commands":{{"c":{{"label":"{}C"}}}}}}}}"#, esc(0x202E)), "`translations.de.commands.c.label` must be"),
            (format!(r#"{{"de":{{"params":{{"n":{{"label":"{}N"}}}}}}}}"#, esc(0x202D)), "`translations.de.params.n.label` must be"),
            (format!(r#"{{"de":{{"params":{{"mode":{{"options":{{"a":"{}A"}}}}}}}}}}"#, esc(0x202B)), "`translations.de.params.mode.options.a`"),
            (format!(r#"{{"de":{{"groups":{{"G":"{}Ge"}}}}}}"#, esc(0x2066)), "`translations.de.groups.G` must be"),
            (format!(r#"{{"de{}":{{"name":"X"}}}}"#, esc(0x202E)), "`translations.de\\u{202E}` is not a language code"),
        ] {
            let e = Manifest::parse(with(&translations).as_bytes()).unwrap_err().to_string();
            assert!(e.contains(says), "{translations}: {e}");
        }
        // An option value holding U+202E shows as the text `\u{202E}`, so a translation giving
        // that text for another option shows the same label.
        let escaped = format!(
            r#"{{"id":"a","name":"x","kind":"filter","params":{{"mode":{{"type":"choice","options":["soft","{}"]}}}},
                "translations":{{"de":{{"params":{{"mode":{{"options":{{"soft":"\\u{{202E}}"}}}}}}}}}}}}"#,
            esc(0x202E)
        );
        let e = Manifest::parse(escaped.as_bytes()).unwrap_err().to_string();
        assert!(e.contains("`translations.de.params.mode.options` gives two options the same label"), "{e}");
        // Keys and types serde reads: an unknown key, a repeated one, and a value of the wrong type.
        for (translations, says) in [
            (r#"{"de":{"title":"X"}}"#, "unknown field `title`"),
            (r#"{"de":{"params":{"n":{"unit":"mm"}}}}"#, "unknown field `unit`"),
            (r#"{"de":{"name":"X","name":"Y"}}"#, "duplicate field `name`"),
            (r#"{"de":{"params":{"n":{"label":"A"},"n":{"label":"B"}}}}"#, "`n` is given twice"),
            (r#"{"de":{"name":"X"},"de":{"name":"Y"}}"#, "`de` is given twice"),
            (r#"{"de":{"name":7}}"#, "manifest"),
            (r#"["de"]"#, "manifest"),
            (r#""de""#, "manifest"),
            ("null", "expected an object"),
            (r#"{"de":null}"#, "manifest"),
            (r#"{"de":{"params":null}}"#, "expected an object"),
            (r#"{"de":{"groups":null}}"#, "expected an object"),
            (r#"{"de":{"commands":null}}"#, "expected an object"),
            (r#"{"de":{"params":{"mode":{"options":null}}}}"#, "expected an object"),
        ] {
            let e = Manifest::parse(with(translations).as_bytes()).unwrap_err().to_string();
            assert!(e.contains(says), "{translations}: {e}");
        }
        // Right-to-left text without direction characters is text like any other.
        let arabic = Manifest::parse(with(r#"{"ar":{"name":"ترتيب","commands":{"c":{"label":"تكبير"}}}}"#).as_bytes()).unwrap();
        assert_eq!(arabic.name_in("ar"), "ترتيب");
    }

    /// At most 16 languages, and the whole manifest, translations included, stays within the
    /// manifest's byte limit.
    #[test]
    fn languages_and_manifest_size_are_capped() {
        const CODES: [&str; 17] = ["de", "fr", "es", "it", "pt", "nl", "sv", "da", "fi", "pl", "cs", "hu", "ro", "tr", "ru", "uk", "ja"];
        let manifest = |n: usize| {
            let langs: Vec<String> = CODES.iter().take(n).map(|c| format!(r#""{c}":{{"name":"X"}}"#)).collect();
            format!(r#"{{"id":"a","name":"x","kind":"filter","translations":{{{}}}}}"#, langs.join(","))
        };
        assert_eq!(Manifest::parse(manifest(MAX_LANGUAGES).as_bytes()).unwrap().translations.0.len(), MAX_LANGUAGES);
        let e = Manifest::parse(manifest(MAX_LANGUAGES + 1).as_bytes()).unwrap_err().to_string();
        assert!(e.contains("`translations` gives at most 16 languages"), "{e}");
        let help = "h".repeat(crate::manifest::MAX_MANIFEST_BYTES);
        let big = format!(
            r#"{{"id":"a","name":"x","kind":"filter","params":{{"n":{{"type":"bool","help":"H"}}}},"translations":{{"de":{{"params":{{"n":{{"help":"{help}"}}}}}}}}}}"#
        );
        assert!(Manifest::parse(big.as_bytes()).unwrap_err().to_string().contains("limit"), "the manifest's byte limit applies");
    }

    #[test]
    fn language_codes() {
        for good in ["de", "DE", "pt-br", "pt-BR", "zh-Hant", "zh-Hant-TW", "es-419", "fil", "sr-Latn-RS"] {
            assert!(valid_language_code(good), "{good}");
        }
        let long = format!("de-{}", ["abcdefgh"; 4].join("-"));
        for bad in ["", "d", "deutsch", "de-", "-de", "de--at", "de_AT", "de-a", "x-test", "i-klingon", "de-abcdefghi", "1d", long.as_str()] {
            assert!(!valid_language_code(bad), "{bad}");
        }
    }
}
