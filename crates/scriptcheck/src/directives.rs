//! Preprocessor lines (`#target acme-12`, `//@include "lib.jsxinc"`): their names and values, the
//! parts of a target's application specifier, and the folders an `#includepath` line names.

use crate::catalog;

/// A preprocessor line.
pub(crate) struct Line<'s> {
    /// `#` or `//@`.
    pub form: &'static str,
    /// The name as written.
    pub name: &'s str,
    /// The documented directive the name is (compared without case), if it is one.
    pub known: Option<&'static str>,
    /// Its value: the text after the name, without quotes around it, or else without a trailing
    /// `//` comment and `;`.
    pub value: &'s str,
}

/// The line `text`, which starts with `#` or `//@`.
pub(crate) fn parse(text: &str) -> Line<'_> {
    let (form, rest) = match text.strip_prefix("//@") {
        Some(rest) => ("//@", rest),
        None => ("#", text.strip_prefix('#').unwrap_or(text)),
    };
    let end = rest.find(|c: char| !c.is_ascii_alphanumeric() && c != '_').unwrap_or(rest.len());
    let (name, rest) = (rest.get(..end).unwrap_or(rest), rest.get(end..).unwrap_or_default());
    let known = catalog::DIRECTIVES.iter().copied().find(|d| d.eq_ignore_ascii_case(name));
    Line { form, name, known, value: value(rest) }
}

fn value(rest: &str) -> &str {
    let v = rest.trim();
    if let Some(quote) = v.chars().next().filter(|c| matches!(c, '"' | '\'')) {
        let inner = v.get(1..).unwrap_or_default();
        return inner.find(quote).and_then(|i| inner.get(..i)).unwrap_or(inner);
    }
    let v = v.find("//").and_then(|i| v.get(..i)).unwrap_or(v);
    v.trim_end().trim_end_matches(';').trim_end()
}

/// The parts of an application specifier, `name_instance-version-locale`, every part after the name
/// optional, as
/// https://extendscript.docsforadobe.dev/interapplication-communication/application-and-namespace-specifiers/
/// lays it out.
pub(crate) struct Specifier<'s> {
    pub app: &'s str,
    pub instance: Option<&'s str>,
    pub version: Option<&'s str>,
    pub locale: Option<&'s str>,
}

pub(crate) fn target(value: &str) -> Specifier<'_> {
    let (head, tail) = match value.split_once('-') {
        Some((head, tail)) => (head, Some(tail)),
        None => (value, None),
    };
    let (app, instance) = match head.split_once('_') {
        Some((app, instance)) => (app, Some(instance)),
        None => (head, None),
    };
    let (version, locale) = match tail.map(|t| t.split_once('-').map_or((t, None), |(v, l)| (v, Some(l)))) {
        Some((version, locale)) => (Some(version), locale),
        None => (None, None),
    };
    fn part(s: Option<&str>) -> Option<&str> {
        s.filter(|s| !s.is_empty())
    }
    Specifier { app, instance: part(instance), version: part(version), locale: part(locale) }
}

/// The most folders an `#includepath` line names that are kept.
const MAX_PATHS: usize = 64;

/// The longest `#include` name or `#includepath` folder followed, in bytes.
pub(crate) const MAX_PATH: usize = 4096;

/// The folders an `#includepath` value names, separated by `;`: the first [`MAX_PATHS`] of those
/// no longer than [`MAX_PATH`].
pub(crate) fn include_paths(value: &str) -> Vec<String> {
    value
        .split(';')
        .map(|p| p.trim().trim_matches(['"', '\'']))
        .filter(|p| !p.is_empty() && p.len() <= MAX_PATH)
        .take(MAX_PATHS)
        .map(str::to_string)
        .collect()
}
