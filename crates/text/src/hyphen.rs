//! Pattern-free English hyphenation: break long words between syllables at vowel–consonant
//! boundaries (V-CV: "ty-po", and VC-CV: "hap-pen"). A paragraph's Hyphenation options
//! ([`HyphenationOptions`]) set the shortest word that breaks, the fewest letters left before and
//! after the hyphen ([`MIN_BEFORE`] and [`MIN_AFTER`] by default), and whether capitalized words
//! break.
//!
//! Preferences › Hyphenation › Exceptions (comma separated) override the points for named words:
//! `typography` never breaks, `ty-pog-ra-phy` breaks only where the hyphens mark. See
//! [`set_hyphenation_exceptions`].

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};

use vectorcraft_doc::HyphenationOptions;

/// Only words at least this long are hyphenated (Illustrator's default: longer than 5 letters).
pub const MIN_WORD: usize = 6;
pub const MIN_BEFORE: usize = 3;
pub const MIN_AFTER: usize = 3;

/// An exception for one word: never break it, or break only at the listed character indices.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Exception {
    Never,
    Only(Vec<usize>),
}

#[derive(Default)]
struct Exceptions {
    source: String,
    map: HashMap<String, Exception>,
}

fn store() -> MutexGuard<'static, Exceptions> {
    static LOCK: OnceLock<Mutex<Exceptions>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(Exceptions::default())).lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn parse_list(source: &str) -> Exceptions {
    let mut map = HashMap::new();
    for entry in source.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let (word, ex) = parse_exception(entry);
        if word.is_empty() {
            continue;
        }
        map.insert(word, ex);
    }
    Exceptions { source: source.to_string(), map }
}

/// Preferences › Hyphenation › Exceptions: comma-separated words. A word with no hyphens is never
/// hyphenated; hyphens mark the only allowed break points (`ty-pog-ra-phy`). Matching is
/// case-insensitive. Empty clears the list.
pub fn set_hyphenation_exceptions(source: &str) {
    *store() = parse_list(source);
}

/// The last string passed to [`set_hyphenation_exceptions`] (empty when none).
pub fn hyphenation_exceptions() -> String {
    store().source.clone()
}

fn parse_exception(entry: &str) -> (String, Exception) {
    // Illustrator: bare word = never break; hyphens mark the only allowed breaks. No other syntax.
    if !entry.contains('-') {
        return (normalize_word(entry), Exception::Never);
    }
    let mut word = String::new();
    let mut points = Vec::new();
    for ch in entry.chars() {
        if ch == '-' {
            let at = word.chars().count();
            if at > 0 {
                points.push(at);
            }
        } else {
            for c in ch.to_lowercase() {
                word.push(c);
            }
        }
    }
    let n = word.chars().count();
    points.retain(|&p| p > 0 && p < n);
    points.sort_unstable();
    points.dedup();
    (word, Exception::Only(points))
}

/// Exception points count chars of the lowercased word; lowercasing can add chars ('İ' becomes
/// 'i' + U+0307), so map each point back to the char index in `word` it falls before. Points
/// inside one char's lowercase expansion are dropped; a point at the end of the word is kept.
fn lowercase_points_to_word(word: &str, pts: &[usize]) -> Vec<usize> {
    let mut out = Vec::with_capacity(pts.len());
    let mut lower = 0;
    let mut len = 0;
    for (i, ch) in word.chars().enumerate() {
        if pts.contains(&lower) {
            out.push(i);
        }
        lower += ch.to_lowercase().count();
        len = i + 1;
    }
    if pts.contains(&lower) {
        out.push(len);
    }
    out
}

fn normalize_word(word: &str) -> String {
    word.chars().flat_map(|c| c.to_lowercase()).collect()
}

/// Look the word up under the preferences lock. Only the matched [`Exception`] is cloned, never
/// the whole list.
#[cfg(not(test))]
fn exception_for(word: &str) -> Option<Exception> {
    store().map.get(&normalize_word(word)).cloned()
}

/// Unit tests read only their own thread's list: tests running in parallel set the process-wide
/// one.
#[cfg(test)]
fn exception_for(word: &str) -> Option<Exception> {
    TEST_OVERRIDE.with(|c| c.borrow().as_ref().and_then(|e| e.map.get(&normalize_word(word)).cloned()))
}

fn is_vowel(c: char) -> bool {
    matches!(c.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u' | 'y') || (c.is_alphabetic() && !c.is_ascii())
}

/// Consonant pairs that stay together at the start of a syllable ("gra-phy", "tea-cher").
fn onset_pair(a: char, b: char) -> bool {
    let (a, b) = (a.to_ascii_lowercase(), b.to_ascii_lowercase());
    matches!(
        (a, b),
        ('c' | 's' | 't' | 'p' | 'w' | 'g', 'h')
            | ('b' | 'c' | 'd' | 'f' | 'g' | 'k' | 'p' | 't', 'r')
            | ('b' | 'c' | 'f' | 'g' | 'k' | 'p' | 's', 'l')
            | ('q', 'u')
    )
}

/// Allowed hyphenation points in `word`, as char indices (a hyphen goes *before* the char), with
/// the default Hyphenation options.
pub fn hyphen_points(word: &str) -> Vec<usize> {
    hyphen_points_with(word, &HyphenationOptions::default())
}

/// Allowed hyphenation points in `word`, as char indices (a hyphen goes *before* the char), within
/// a paragraph's Hyphenation options: words with more letters than Words Longer Than, at least
/// After First letters before the hyphen and Before Last after it (at least 1 each), and no word
/// that starts with a capital when Hyphenate Capitalized Words is off. Words containing non-letters
/// (other than a trailing apostrophe) are not hyphenated. Preferences exceptions override the
/// pattern and these options for a matching word.
pub fn hyphen_points_with(word: &str, opts: &HyphenationOptions) -> Vec<usize> {
    if let Some(ex) = exception_for(word) {
        return match ex {
            Exception::Never => vec![],
            Exception::Only(pts) => lowercase_points_to_word(word, &pts),
        };
    }
    let c: Vec<char> = word.chars().collect();
    let n = c.len();
    let longer_than = usize::try_from(opts.words_longer_than).unwrap_or(usize::MAX);
    if n <= longer_than || !c.iter().all(|c| c.is_alphabetic() || matches!(c, '\'' | '’')) {
        return vec![];
    }
    // A capitalised run like "NASA" or a proper name stays whole only when all caps.
    if c.iter().all(|c| c.is_uppercase()) {
        return vec![];
    }
    if !opts.capitalized && c.first().is_some_and(|c| c.is_uppercase()) {
        return vec![];
    }
    let before = usize::try_from(opts.after_first).unwrap_or(usize::MAX).max(1);
    let after = usize::try_from(opts.before_last).unwrap_or(usize::MAX).max(1);
    let v: Vec<bool> = c.iter().map(|&x| is_vowel(x)).collect();
    let mut out = vec![];
    for i in before..=n.saturating_sub(after) {
        if !c[i].is_alphabetic() || !c[i - 1].is_alphabetic() {
            continue;
        }
        let ok = if v[i - 1] && !v[i] {
            // V-CV: a single consonant (or an onset pair) followed by a vowel starts the syllable.
            (i + 1 < n && v[i + 1]) || (i + 2 < n && onset_pair(c[i], c[i + 1]) && v[i + 2])
        } else if !v[i - 1] && !v[i] {
            // VC-CV: split a consonant cluster after the first consonant unless it's an onset pair.
            i >= 2 && v[i - 2] && i + 1 < n && v[i + 1] && !onset_pair(c[i - 1], c[i])
        } else {
            false
        };
        if ok {
            out.push(i);
        }
    }
    out
}

/// `word` with `-` at every hyphenation point (for tests and diagnostics).
pub fn hyphenate_word(word: &str) -> String {
    let pts = hyphen_points(word);
    let mut s = String::with_capacity(word.len() + pts.len());
    for (i, ch) in word.chars().enumerate() {
        if pts.contains(&i) {
            s.push('-');
        }
        s.push(ch);
    }
    s
}

#[cfg(test)]
thread_local! {
    /// Per-thread list so unit tests don't race the process-wide preferences store.
    static TEST_OVERRIDE: std::cell::RefCell<Option<Exceptions>> = const { std::cell::RefCell::new(None) };
}

/// Run `f` with Preferences › Hyphenation › Exceptions applied on this thread only (tests).
#[cfg(test)]
pub(crate) fn with_hyphenation_exceptions_for_test(source: &str, f: impl FnOnce()) {
    TEST_OVERRIDE.with(|c| *c.borrow_mut() = Some(parse_list(source)));
    f();
    TEST_OVERRIDE.with(|c| *c.borrow_mut() = None);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_exceptions(source: &str, f: impl FnOnce()) {
        with_hyphenation_exceptions_for_test(source, f);
    }

    /// #394 Preferences › Hyphenation › Exceptions contract (parse + override).
    #[test]
    fn hyphenation_exceptions_override_the_pattern() {
        let pattern = hyphen_points("typography");
        assert_eq!(pattern, vec![4, 7], "pattern: typo-gra-phy");

        // Bare word: never break (case-insensitive).
        with_exceptions("typography", || {
            assert!(hyphen_points("typography").is_empty());
            assert!(hyphen_points("Typography").is_empty(), "case-insensitive");
        });

        // Hyphens mark the only allowed breaks (stricter than the pattern).
        with_exceptions("typo-graphy", || {
            assert_eq!(hyphen_points("typography"), vec![4], "only the marked break");
            assert_eq!(hyphenate_word("typography"), "typo-graphy");
        });

        // Comma-separated list; empty / whitespace entries ignored.
        with_exceptions("  , typography , hap-pen ,  ", || {
            assert!(hyphen_points("typography").is_empty());
            assert_eq!(hyphen_points("happen"), vec![3]);
            assert!(!hyphen_points("hyphenation").is_empty(), "unlisted words still use the pattern");
        });

        assert_eq!(hyphen_points("typography"), pattern, "override cleared");
    }

    /// Exception break points are char indices into the word itself, even when lowercasing
    /// changes the char count ('İ' lowercases to 'i' + U+0307).
    #[test]
    fn exception_points_index_the_original_word() {
        with_exceptions("İs-tan-bul", || {
            assert_eq!(hyphen_points("İstanbul"), vec![2, 5]);
            assert_eq!(hyphenate_word("İstanbul"), "İs-tan-bul");
            assert_eq!(hyphenate_word("İSTANBUL"), "İS-TAN-BUL");
        });
    }

    /// The default Hyphenation options are the limits of [`hyphen_points`]: words of 6 letters or
    /// more, 3 letters on each side of the hyphen.
    #[test]
    fn default_hyphenation_options_keep_the_default_limits() {
        let d = HyphenationOptions::default();
        assert_eq!((d.words_longer_than as usize + 1, d.after_first as usize, d.before_last as usize), (MIN_WORD, MIN_BEFORE, MIN_AFTER));
        for w in ["typography", "happen", "hyphenation", "language", "aromatic", "Language", "short"] {
            assert_eq!(hyphen_points_with(w, &d), hyphen_points(w), "{w}");
        }
    }

    /// Paragraph panel menu › Hyphenation…: Words Longer Than, After First, Before Last and
    /// Hyphenate Capitalized Words limit where a word breaks.
    #[test]
    fn hyphenation_options_limit_the_points() {
        let opts = |f: &dyn Fn(&mut HyphenationOptions)| {
            let mut o = HyphenationOptions::default();
            f(&mut o);
            o
        };
        // An 8-letter word breaks by default, and not when only longer words may.
        assert_eq!(hyphen_points("language"), vec![3]);
        assert!(hyphen_points_with("language", &opts(&|o| o.words_longer_than = 10)).is_empty());
        assert!(hyphen_points_with("language", &opts(&|o| o.words_longer_than = 8)).is_empty());
        assert_eq!(hyphen_points_with("language", &opts(&|o| o.words_longer_than = 7)), vec![3]);
        // After First and Before Last: "aro-ma-tic" by default, nothing with 4 and 4, "a-ro-ma-tic" with 1 and 1.
        assert_eq!(hyphen_points("aromatic"), vec![3, 5]);
        assert!(hyphen_points_with("aromatic", &opts(&|o| (o.after_first, o.before_last) = (4, 4))).is_empty());
        assert_eq!(hyphen_points_with("aromatic", &opts(&|o| (o.after_first, o.before_last) = (1, 1))), vec![1, 3, 5]);
        // 0 letters on either side still leaves the hyphen inside the word.
        assert_eq!(hyphen_points_with("aromatic", &opts(&|o| (o.after_first, o.before_last) = (0, 0))), vec![1, 3, 5]);
        // Capitalized words.
        assert_eq!(hyphen_points("Language"), vec![3]);
        assert!(hyphen_points_with("Language", &opts(&|o| o.capitalized = false)).is_empty());
        assert_eq!(hyphen_points_with("language", &opts(&|o| o.capitalized = false)), vec![3]);
    }

    /// Process-wide store used by `prefs.set` / restore.
    #[test]
    fn set_hyphenation_exceptions_round_trips_the_source_string() {
        set_hyphenation_exceptions("typography, hap-pen");
        assert_eq!(hyphenation_exceptions(), "typography, hap-pen");
        set_hyphenation_exceptions("");
        assert!(hyphenation_exceptions().is_empty());
    }
}
