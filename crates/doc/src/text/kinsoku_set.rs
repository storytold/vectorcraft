//! Type › Kinsoku Shori Settings…: a custom Kinsoku Shori set, saved with the document. Its four
//! categories hold the characters that may not start a line, those that may not end one, the
//! hanging punctuation that Burasagari lets stand outside a line, and the inseparable sequences
//! that Bunri-Kinshi keeps on one line. A paragraph that uses a set holds a copy of it
//! ([`super::ParaStyle::kinsoku_set`]).
//!
//! Behavior reference: the help page
//! <https://helpx.adobe.com/illustrator/desktop/design-with-text/edit-format-text/use-kinsoku-for-japanese-line-breaking.html>.
//! The name rules and the limits are VectorCraft's own.

use serde::{Deserialize, Serialize};

/// The longest set name, in characters.
pub const KINSOKU_NAME_MAX: usize = 128;
/// The most custom sets a document keeps.
pub const KINSOKU_SETS_MAX: usize = 256;
/// The most characters one category holds (an inseparable sequence counts each of its characters).
pub const KINSOKU_CATEGORY_MAX: usize = 4096;
/// The fewest and most characters of an inseparable sequence.
pub const KINSOKU_SEQUENCE: std::ops::RangeInclusive<usize> = 2..=16;
/// The names of the built-in sets, which no custom set may take (in any case).
pub const KINSOKU_BUILT_IN: [&str; 3] = ["None", "Hard", "Soft"];

/// A custom Kinsoku Shori set. Each list of characters is sorted and holds each character once;
/// the inseparable sequences are sorted and unique.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "KinsokuSetFile", into = "KinsokuSetFile")]
pub struct KinsokuSet {
    pub name: String,
    /// Cannot Start Line: characters that never start a line.
    pub cannot_start: Vec<char>,
    /// Cannot End Line: characters that never end a line.
    pub cannot_end: Vec<char>,
    /// Hanging Punctuation: characters that Burasagari lets hang outside a line's end.
    pub hanging: Vec<char>,
    /// Bunri-Kinshi: sequences of characters that a line never breaks inside while Bunri-Kinshi is
    /// on.
    pub inseparable: Vec<String>,
}

/// [`KinsokuSet`] as saved: each category of single characters as one string.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KinsokuSetFile {
    #[serde(default, deserialize_with = "read_name")]
    name: String,
    #[serde(default, deserialize_with = "read_category")]
    cannot_start: String,
    #[serde(default, deserialize_with = "read_category")]
    cannot_end: String,
    #[serde(default, deserialize_with = "read_category")]
    hanging: String,
    #[serde(default, deserialize_with = "read_sequences")]
    inseparable: Vec<String>,
}

/// The most characters of a name read from a file, spaces around it included; a longer name is
/// not read.
const NAME_READ_MAX: usize = KINSOKU_CATEGORY_MAX;
/// The most characters of a category read from a file, before control and space characters are
/// left out.
const CATEGORY_READ_MAX: usize = KINSOKU_CATEGORY_MAX * 4;
/// The longest inseparable sequence, as a constant for [`Capped`].
const SEQUENCE_MAX: usize = *KINSOKU_SEQUENCE.end();

/// Text read from a file, copied up to `N` characters: a longer text keeps its first `N + 1`, which
/// no rule that allows `N` takes, and is not copied whole.
struct Capped<const N: usize>(String);

impl<'de, const N: usize> Deserialize<'de> for Capped<N> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Text<const N: usize>;
        impl<const N: usize> serde::de::Visitor<'_> for Text<N> {
            type Value = Capped<N>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("text")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Capped<N>, E> {
                Ok(Capped(v.chars().take(N + 1).collect()))
            }
        }
        d.deserialize_str(Text::<N>)
    }
}

/// A set's name as read from a file: empty, so not read, when it is longer than [`NAME_READ_MAX`].
fn read_name<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let Capped(name) = Capped::<NAME_READ_MAX>::deserialize(d)?;
    Ok(if name.chars().count() > NAME_READ_MAX { String::new() } else { name })
}

/// A category of a set as read from a file: its first [`CATEGORY_READ_MAX`] characters.
fn read_category<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let Capped(text) = Capped::<CATEGORY_READ_MAX>::deserialize(d)?;
    Ok(text.chars().take(CATEGORY_READ_MAX).collect())
}

/// Reads a list, keeping the items `keep` keeps up to `max` of them and skipping the rest of a
/// longer list without storing it.
struct Kept<T, F>(usize, F, std::marker::PhantomData<T>);

impl<'de, T: Deserialize<'de>, F: FnMut(&T) -> bool> serde::de::Visitor<'de> for Kept<T, F> {
    type Value = Vec<T>;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a list")
    }
    fn visit_seq<A: serde::de::SeqAccess<'de>>(mut self, mut seq: A) -> Result<Vec<T>, A::Error> {
        let mut out = Vec::new();
        while out.len() < self.0 {
            match seq.next_element::<T>()? {
                Some(item) if (self.1)(&item) => out.push(item),
                Some(_) => {}
                None => return Ok(out),
            }
        }
        while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
        Ok(out)
    }
}

/// A set's inseparable sequences as read from a file: those [`KinsokuSet::takes_sequence`] takes,
/// at most [`KINSOKU_CATEGORY_MAX`]. A text longer than a sequence is not copied whole.
fn read_sequences<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let keep = |s: &Capped<SEQUENCE_MAX>| KinsokuSet::takes_sequence(&s.0);
    let kept = d.deserialize_seq(Kept(KINSOKU_CATEGORY_MAX, keep, std::marker::PhantomData))?;
    Ok(kept.into_iter().map(|Capped(s)| s).collect())
}

impl From<KinsokuSet> for KinsokuSetFile {
    fn from(s: KinsokuSet) -> Self {
        Self {
            name: s.name,
            cannot_start: s.cannot_start.into_iter().collect(),
            cannot_end: s.cannot_end.into_iter().collect(),
            hanging: s.hanging.into_iter().collect(),
            inseparable: s.inseparable,
        }
    }
}

impl From<KinsokuSetFile> for KinsokuSet {
    /// A set read from a file keeps what a set can hold: its name trimmed, or no name when
    /// [`KinsokuSet::clean_name`] refuses it; each category's characters sorted and unique, without
    /// control or space characters, up to [`KINSOKU_CATEGORY_MAX`]; and the inseparable sequences of
    /// [`KINSOKU_SEQUENCE`] characters, up to the same count of characters.
    fn from(f: KinsokuSetFile) -> Self {
        let chars = |s: &str| s.chars().filter(|&c| KinsokuSet::takes_char(c)).take(KINSOKU_CATEGORY_MAX * 4).collect::<Vec<char>>();
        let mut set = KinsokuSet {
            name: KinsokuSet::clean_name(&f.name).unwrap_or_default(),
            cannot_start: chars(&f.cannot_start),
            cannot_end: chars(&f.cannot_end),
            hanging: chars(&f.hanging),
            inseparable: f.inseparable,
        };
        set.normalize();
        set.cannot_start.truncate(KINSOKU_CATEGORY_MAX);
        set.cannot_end.truncate(KINSOKU_CATEGORY_MAX);
        set.hanging.truncate(KINSOKU_CATEGORY_MAX);
        let mut total = 0;
        set.inseparable.retain(|s| {
            total += s.chars().count();
            total <= KINSOKU_CATEGORY_MAX
        });
        set
    }
}

/// A document's custom sets as read from a file: each set with a name, the first set of each name
/// (in any case), at most [`KINSOKU_SETS_MAX`]; the sets past those are skipped without being
/// stored.
pub(crate) fn read_sets<'de, D: serde::Deserializer<'de>>(d: D) -> Result<crate::Shared<Vec<KinsokuSet>>, D::Error> {
    let mut seen = std::collections::HashSet::new();
    let keep = move |s: &KinsokuSet| !s.name.is_empty() && seen.insert(s.name.to_lowercase());
    Ok(crate::Shared::new(d.deserialize_seq(Kept(KINSOKU_SETS_MAX, keep, std::marker::PhantomData))?))
}

impl KinsokuSet {
    /// Do `a` and `b` name the same set: equal in any case?
    pub fn same_name(a: &str, b: &str) -> bool {
        a == b || a.to_lowercase() == b.to_lowercase()
    }

    /// The set of `sets` (a document's custom sets) named `name`, in any case.
    pub fn named<'a>(sets: &'a [KinsokuSet], name: &str) -> Option<&'a KinsokuSet> {
        sets.iter().find(|s| Self::same_name(&s.name, name))
    }

    /// `name` trimmed, when it can name a custom set: not empty, at most [`KINSOKU_NAME_MAX`]
    /// characters, without control characters, and none of [`KINSOKU_BUILT_IN`] in any case.
    pub fn clean_name(name: &str) -> Option<String> {
        let name = name.trim();
        let ok = !name.is_empty()
            && name.chars().count() <= KINSOKU_NAME_MAX
            && !name.chars().any(char::is_control)
            && !KINSOKU_BUILT_IN.iter().any(|b| Self::same_name(b, name));
        ok.then(|| name.to_string())
    }

    /// Can a category hold character `c`: not a control character nor a space?
    pub fn takes_char(c: char) -> bool {
        !c.is_control() && !c.is_whitespace()
    }

    /// Can `s` be an inseparable sequence: [`KINSOKU_SEQUENCE`] characters that
    /// [`Self::takes_char`] takes?
    pub fn takes_sequence(s: &str) -> bool {
        KINSOKU_SEQUENCE.contains(&s.chars().count()) && s.chars().all(Self::takes_char)
    }

    /// Sort each category and keep each character or sequence once.
    pub fn normalize(&mut self) {
        for list in [&mut self.cannot_start, &mut self.cannot_end, &mut self.hanging] {
            list.sort_unstable();
            list.dedup();
        }
        self.inseparable.sort_unstable();
        self.inseparable.dedup();
    }

    /// Why this set can't be used, if it can't: a name [`Self::clean_name`] refuses, a character
    /// [`Self::takes_char`] refuses, a sequence [`Self::takes_sequence`] refuses, or a category of
    /// more than [`KINSOKU_CATEGORY_MAX`] characters.
    pub fn check(&self) -> Result<(), String> {
        if Self::clean_name(&self.name).as_deref() != Some(self.name.as_str()) {
            return Err(format!(
                "`{}` can't name a Kinsoku set (1 to {KINSOKU_NAME_MAX} characters, no control characters, not None, Hard or Soft)",
                self.name
            ));
        }
        for (key, list) in [("cannotStart", &self.cannot_start), ("cannotEnd", &self.cannot_end), ("hanging", &self.hanging)] {
            if list.len() > KINSOKU_CATEGORY_MAX {
                return Err(format!("{key}: at most {KINSOKU_CATEGORY_MAX} characters"));
            }
            if let Some(c) = list.iter().find(|&&c| !Self::takes_char(c)) {
                return Err(format!("{key}: U+{:04X} is a control or space character", u32::from(*c)));
            }
        }
        if let Some(s) = self.inseparable.iter().find(|s| !Self::takes_sequence(s)) {
            return Err(format!(
                "inseparable: `{s}` must be {} to {} characters, none of them control or space characters",
                KINSOKU_SEQUENCE.start(),
                KINSOKU_SEQUENCE.end()
            ));
        }
        if self.inseparable.iter().map(|s| s.chars().count()).sum::<usize>() > KINSOKU_CATEGORY_MAX {
            return Err(format!("inseparable: at most {KINSOKU_CATEGORY_MAX} characters in all"));
        }
        Ok(())
    }

    /// May `c` not start a line?
    pub fn cannot_start(&self, c: char) -> bool {
        self.cannot_start.binary_search(&c).is_ok()
    }

    /// May `c` not end a line?
    pub fn cannot_end(&self, c: char) -> bool {
        self.cannot_end.binary_search(&c).is_ok()
    }

    /// May `c` hang outside a line's end (Burasagari)?
    pub fn hangs(&self, c: char) -> bool {
        self.hanging.binary_search(&c).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{Kinsoku, ParaStyle};
    use serde_json::json;
    use std::sync::Arc;

    /// A set is saved with each category as one string and read back as it was; a paragraph saves
    /// its copy of the set beside Hard, and a paragraph saved without one reads with none.
    #[test]
    fn kinsoku_sets_are_saved_with_the_paragraphs_that_use_them() {
        let mut set = KinsokuSet {
            name: "Tight".into(),
            cannot_start: "」。、".chars().collect(),
            cannot_end: vec!['「'],
            hanging: vec!['、', '。'],
            inseparable: vec!["——".into(), "あいう".into()],
        };
        set.normalize();
        let saved = serde_json::to_value(&set).unwrap();
        assert_eq!(saved, json!({"name": "Tight", "cannotStart": "、。」", "cannotEnd": "「", "hanging": "、。", "inseparable": ["——", "あいう"]}));
        assert_eq!(serde_json::from_value::<KinsokuSet>(saved).unwrap(), set);
        let p = ParaStyle { kinsoku_set: Some(Arc::new(set.clone())), ..ParaStyle::default() };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["kinsoku_set"]["name"], "Tight");
        assert!(v.get("kinsoku").is_none(), "Hard, the default, is left out");
        let back: ParaStyle = serde_json::from_value(v).unwrap();
        assert_eq!(back.kinsoku_set.as_deref(), Some(&set));
        assert_eq!((back.kinsoku, back.kinsoku_name(), back.kinsoku_on()), (Kinsoku::Hard, "Tight", true));
        let old: ParaStyle = serde_json::from_value(json!({"justify": "Left", "kinsoku": "soft"})).unwrap();
        assert_eq!((old.kinsoku_set, old.kinsoku), (None, Kinsoku::Soft));
        assert!(serde_json::to_value(ParaStyle::default()).unwrap().get("kinsoku_set").is_none());
    }

    /// A set read from a file keeps only what a set can hold: each character once, sorted, without
    /// control or space characters, at most 4096 to a category, and sequences of 2 to 16
    /// characters; a category left out is empty. Only the start of a longer text is read.
    #[test]
    fn kinsoku_sets_read_from_a_file_keep_what_a_set_can_hold() {
        let long: String = (0x4E00..0x4E00 + 5000).filter_map(char::from_u32).collect();
        let v = json!({
            "name": "Odd",
            "cannotStart": "ccba \t\u{0}a",
            "cannotEnd": long,
            "inseparable": ["a", "ab", "ab", "a b", "abcdefghijklmnopq", "abcdefghijklmnop"],
        });
        let set: KinsokuSet = serde_json::from_value(v).unwrap();
        assert_eq!(set.cannot_start, ['a', 'b', 'c']);
        assert_eq!(set.cannot_end.len(), KINSOKU_CATEGORY_MAX);
        assert!(set.hanging.is_empty());
        assert_eq!(set.inseparable, ["ab", "abcdefghijklmnop"]);
        assert!(set.check().is_ok());
        let empty: KinsokuSet = serde_json::from_value(json!({})).unwrap();
        assert!(empty.check().is_err(), "a set needs a name");
        // Only the start of a long text is read: a category's first 16384 characters, a name of at
        // most 4096 characters, spaces included, and a sequence's first 17.
        let spaces = " ".repeat(CATEGORY_READ_MAX);
        let text = format!(
            r#"{{"name": "{}Tight", "hanging": "{spaces}、", "cannotEnd": "、", "inseparable": ["{}あ"]}}"#,
            " ".repeat(NAME_READ_MAX),
            "あ".repeat(100)
        );
        let set: KinsokuSet = serde_json::from_str(&text).unwrap();
        assert_eq!((set.name.as_str(), set.hanging.len(), set.cannot_end.len(), set.inseparable.len()), ("", 0, 1, 0));
        let set: KinsokuSet = serde_json::from_str(&format!(r#"{{"name": "{}Tight"}}"#, " ".repeat(NAME_READ_MAX - 5))).unwrap();
        assert_eq!(set.name, "Tight");
    }

    /// A set read from a file under a name a custom set can't take (a built-in name, a blank one)
    /// is not read: a paragraph holding it keeps its Kinsoku Set, so it doesn't show as Hard while
    /// composing by other characters, and a document's list drops it. The list also keeps the
    /// first set of each name in any case, and a name read with spaces around it is trimmed. A
    /// paragraph's set read beside None or Soft is not read either.
    #[test]
    fn kinsoku_sets_read_under_names_a_set_cannot_take_are_dropped() {
        let p: ParaStyle = serde_json::from_value(json!({"kinsoku_set": {"name": "Hard", "cannotStart": "あ"}})).unwrap();
        assert_eq!((p.kinsoku_set, p.kinsoku), (None, Kinsoku::Hard));
        let p: ParaStyle = serde_json::from_value(json!({"kinsoku": "none", "kinsoku_set": {"name": "Tight", "cannotStart": "あ"}})).unwrap();
        assert_eq!((p.kinsoku_set.is_none(), p.kinsoku_name()), (true, "None"));
        let p: ParaStyle = serde_json::from_value(json!({"kinsoku_set": {"name": " Tight ", "cannotStart": "あ"}})).unwrap();
        assert_eq!(p.kinsoku_name(), "Tight");
        let mut v = serde_json::to_value(crate::Document::new(100.0, 100.0)).unwrap();
        v["kinsoku_sets"] = json!([{"name": "soft"}, {"name": "  "}, {"name": "Tight", "cannotEnd": "「"}, {"name": "TIGHT"}, {"name": "Loose"}]);
        let d: crate::Document = serde_json::from_value(v).unwrap();
        let names: Vec<&str> = d.kinsoku_sets.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Tight", "Loose"]);
        assert_eq!(d.kinsoku_sets[0].cannot_end, ['「']);
        let mut v = serde_json::to_value(crate::Document::new(100.0, 100.0)).unwrap();
        // The sets past the limit are skipped unread: one that isn't a set at all doesn't fail the
        // file.
        let mut list: Vec<serde_json::Value> = (0..KINSOKU_SETS_MAX + 5).map(|i| json!({"name": format!("S{i}")})).collect();
        list.push(json!("not a set"));
        v["kinsoku_sets"] = json!(list);
        let d: crate::Document = serde_json::from_value(v).unwrap();
        assert_eq!(d.kinsoku_sets.len(), KINSOKU_SETS_MAX);
        let past: Vec<String> = (0..KINSOKU_CATEGORY_MAX + 1).map(|i| format!("あ{i}")).chain(["x".to_string()]).collect();
        let mut seqs: Vec<serde_json::Value> = past.iter().map(|s| json!(s)).collect();
        seqs.push(json!(7));
        let set: KinsokuSet = serde_json::from_value(json!({"name": "Long", "inseparable": seqs})).unwrap();
        assert!(set.check().is_ok());
    }

    /// A name is trimmed, 1 to 128 characters without control characters, and never None, Hard or
    /// Soft in any case; names match in any case.
    #[test]
    fn kinsoku_set_names_are_checked() {
        assert_eq!(KinsokuSet::clean_name("  Tight "), Some("Tight".into()));
        for bad in ["", "   ", "hard", "SOFT", "None", "a\u{7}b", &"x".repeat(KINSOKU_NAME_MAX + 1)] {
            assert_eq!(KinsokuSet::clean_name(bad), None, "{bad:?}");
        }
        assert!(KinsokuSet::clean_name(&"漢".repeat(KINSOKU_NAME_MAX)).is_some());
        assert!(KinsokuSet::same_name("Tight", "tIGHT"));
        let sets = [KinsokuSet { name: "Tight".into(), ..Default::default() }];
        assert!(KinsokuSet::named(&sets, "TIGHT").is_some() && KinsokuSet::named(&sets, "Loose").is_none());
    }
}
