//! Break points from craft-proof for the languages it has rules or patterns of its own for:
//! German, Polish, Portuguese and Italian (hyph-utf8's TeX patterns, embedded only when craft-proof
//! is built with `CRAFT_I18N_DIR`), Bulgarian and Turkish (syllable rules) and Russian (word-division
//! rules). craft-proof hyphenates any other word as English; those points are dropped here, so a
//! word outside its language's script, or one these rules don't cover, stays whole.

use craft_proof::hyphen::{Patterns, ProofLang, Source, word_breaks_in};
use unicode_normalization::UnicodeNormalization;
use unicode_script::{Script, UnicodeScript};

pub(super) const TAGS: [&str; 7] = ["de", "pl", "pt", "it", "bg", "tr", "ru"];

fn lang(index: usize) -> Option<ProofLang> {
    Some(match *TAGS.get(index)? {
        "de" => ProofLang::De,
        "pl" => ProofLang::Pl,
        "pt" => ProofLang::Pt,
        "it" => ProofLang::It,
        "bg" => ProofLang::Bg,
        "tr" => ProofLang::Tr,
        "ru" => ProofLang::Ru,
        _ => return None,
    })
}

/// The pattern file of a pattern language, `None` for the languages with rules.
fn patterns(lang: ProofLang) -> Option<&'static Patterns> {
    match lang {
        ProofLang::De => Some(Patterns::de()),
        ProofLang::Pl => Some(Patterns::pl()),
        ProofLang::Pt => Some(Patterns::pt()),
        ProofLang::It => Some(Patterns::it()),
        _ => None,
    }
}

/// Whether the language has break points in this build: the rule languages always, the pattern
/// languages when their patterns are embedded.
pub(super) fn available(index: usize) -> bool {
    lang(index).is_some_and(|l| patterns(l).is_none_or(|p| !p.is_empty()))
}

/// A Latin letter or a combining mark.
fn latin(c: char) -> bool {
    matches!(c.script(), Script::Latin | Script::Inherited) && c != '\u{200c}' && c != '\u{200d}'
}

/// A Cyrillic letter, as craft-proof's Russian rules read them.
fn cyrillic(c: char) -> bool {
    matches!(c, '\u{0400}'..='\u{052F}') && c.is_alphabetic()
}

/// Whether `word` is written in the language's script, and the sources of points that come from
/// the language's own rules or patterns. A Cyrillic word is checked composed, as craft-proof
/// hyphenates it: `и` + U+0306 is `й`, while a stress mark, which composes with nothing, is not a
/// letter.
fn fits(lang: ProofLang, word: &str) -> Option<&'static [Source]> {
    let all = |f: fn(char) -> bool| word.chars().all(f);
    let composed_cyrillic = || word.nfc().all(cyrillic);
    match lang {
        ProofLang::De | ProofLang::Pl | ProofLang::Pt | ProofLang::It => all(latin).then_some(&[Source::Patterns, Source::Dictionary]),
        ProofLang::Tr => word.chars().all(|c| latin(c) || matches!(c, '\'' | '’')).then_some(&[Source::Rules]),
        ProofLang::Bg => composed_cyrillic().then_some(&[Source::Rules]),
        ProofLang::Ru => composed_cyrillic().then_some(&[Source::Heuristic]),
        _ => None,
    }
}

/// Character-index candidates for one language, before the paragraph's limits; empty when the
/// word isn't in the language's script or the language's own rules give it none.
pub(super) fn points(word: &str, index: usize) -> Vec<usize> {
    let Some(lang) = lang(index) else { return vec![] };
    let Some(sources) = fits(lang, word) else { return vec![] };
    let (points, source) = word_breaks_in(word, lang);
    if sources.contains(&source) { points } else { vec![] }
}
