//! Words that mean the same thing when searching the command palette: VectorCraft's own groups,
//! written for this table. A query word in a group also finds results whose label or menu path
//! holds another word of its group ([`super::search`]).
//!
//! A word that is the whole label of a command stays out of a group whose other words name a
//! different command: Merge is the label of Pathfinder and Live Paint commands that are not Unite,
//! and Mirror is the label of a repeat that is not Reflect…, so these are in no group. Insert is left
//! out too, since it starts the labels of type commands that are not Place…. A word that only starts
//! or ends other labels can be in a group, and a search for another word of its group also lists
//! those labels: "unite" lists Combine Slices after Unite, and "delete" lists Clear Guides. Plural and
//! verb forms that the search's English stem forms already join (round and rounded) are not groups.
//!
//! A catalog may add a group's words in its language with a row in the `search` context: the group's
//! English words joined by ", ", then that language's words joined by ", ". The words join the group
//! while that language is the UI language. No catalog has to have these rows. The search splits
//! labels into words only at spaces and punctuation, so in a language written without spaces a row's
//! word finds only a label that is that word alone.

/// The groups: lowercase single English words, at most `MAX_WORDS` in a group and `MAX_GROUPS`
/// groups, no word longer than [`MAX_CHARS`] characters and no word in two groups.
pub(crate) const GROUPS: &[&[&str]] = &[
    &["remove", "delete", "erase", "clear"],
    &["background", "bg"],
    &["image", "picture", "photo", "img", "bitmap"],
    &["text", "type"],
    &["reflect", "flip"],
    &["unite", "combine"],
    &["trace", "vectorize"],
    &["color", "colour"],
    &["gray", "grey"],
    &["grayscale", "greyscale"],
    &["center", "centre"],
    &["duplicate", "clone"],
    &["zoom", "magnify"],
    &["rotate", "turn", "spin"],
    &["scale", "resize"],
    &["opacity", "transparency"],
    &["font", "typeface"],
    &["preferences", "settings", "prefs"],
    &["artboard", "canvas"],
    &["swatch", "palette"],
    &["eyedropper", "dropper"],
];

/// The most groups [`GROUPS`] may hold.
#[cfg(test)]
const MAX_GROUPS: usize = 60;
/// The most words a group may hold.
#[cfg(test)]
const MAX_WORDS: usize = 6;
/// The most characters a word may have.
pub(crate) const MAX_CHARS: usize = 20;

/// The context of a catalog row that adds a group's words in its language.
pub(crate) const CONTEXT: &str = "search";

/// The source text of group `group`'s row in a catalog: its words joined by ", ".
pub(crate) fn row_key(group: &[&str]) -> String {
    group.join(", ")
}

/// The words of a catalog row's translation `text`, each trimmed and lowercased with its runs of
/// whitespace made one space, and whether each is a single word of at most [`MAX_CHARS`]
/// characters that the search can look up.
pub(crate) fn row_words(text: &str) -> impl Iterator<Item = (String, bool)> + '_ {
    text.split(',').map(|w| {
        let w = super::search::normalize(w);
        let single = !w.is_empty() && w.chars().count() <= MAX_CHARS && super::search::is_one_word(&w);
        (w, single)
    })
}

/// What is wrong with a catalog's `search` row for `source` translated as `text`: a source that is
/// no group's [`row_key`], an empty word, a word that is not a single word, or a word that is a word
/// of another group.
#[cfg(test)]
pub(crate) fn row_problems(source: &str, text: &str) -> Vec<String> {
    let Some(group) = GROUPS.iter().position(|g| row_key(g) == source) else {
        return vec![format!("{source:?} is not a group")];
    };
    let mut problems = vec![];
    for (w, single) in row_words(text) {
        if w.is_empty() {
            problems.push(format!("{source:?}: an empty word"));
        } else if !single {
            problems.push(format!("{source:?}: {w:?} is not a single word"));
        } else if let Some(other) = GROUPS.iter().enumerate().find(|&(i, g)| i != group && g.contains(&w.as_str())) {
            problems.push(format!("{source:?}: {w:?} is a word of {:?}", row_key(other.1)));
        }
    }
    problems
}

/// What is wrong with one catalog's `search` rows, each a group's `source` and its translation
/// `text`: each row's [`row_problems`], and a word that rows add to two different groups.
#[cfg(test)]
pub(crate) fn catalog_problems<'a>(rows: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<String> {
    let mut problems = vec![];
    let mut groups: std::collections::HashMap<String, &str> = std::collections::HashMap::new();
    for (source, text) in rows {
        problems.extend(row_problems(source, text));
        for (w, _) in row_words(text).filter(|(_, single)| *single) {
            match groups.get(w.as_str()) {
                Some(&first) if first != source => problems.push(format!("{w:?} is a word of {first:?} and {source:?}")),
                Some(_) => {}
                None => {
                    groups.insert(w, source);
                }
            }
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table keeps to its limits: groups, words per group, characters per word, and each word
    /// lowercase, a single word and in one group only.
    #[test]
    fn the_groups_keep_to_their_limits() {
        assert!(GROUPS.len() <= MAX_GROUPS, "{} groups", GROUPS.len());
        let mut seen = std::collections::HashSet::new();
        for group in GROUPS {
            assert!((2..=MAX_WORDS).contains(&group.len()), "{group:?}");
            for &w in *group {
                assert!(w.chars().count() <= MAX_CHARS, "{w}");
                assert_eq!(super::super::search::normalize(w), w, "{w} is lowercase");
                assert!(super::super::search::is_one_word(w), "{w} is one word");
                assert!(seen.insert(w), "{w} is in two groups");
            }
        }
    }

    /// A catalog row passes when it names a group and adds single words of no other group, and fails
    /// otherwise.
    #[test]
    fn catalog_rows_are_checked() {
        assert!(row_problems("remove, delete, erase, clear", "entfernen, löschen").is_empty());
        assert_eq!(row_problems("remove, delete", "entfernen").len(), 1, "not a group's words");
        assert_eq!(row_problems("remove, delete, erase, clear", "entfernen,, löschen").len(), 1, "an empty word");
        assert_eq!(row_problems("remove, delete, erase, clear", "ganz entfernen").len(), 1, "two words");
        assert_eq!(row_problems("remove, delete, erase, clear", "Text").len(), 1, "a word of another group once lowercased");
    }

    /// A catalog's rows fail when they add one word to two groups, and pass when a word is repeated
    /// for one group or the catalog has no rows.
    #[test]
    fn a_word_in_two_groups_of_a_catalog_is_checked() {
        let (remove, artboard) = ("remove, delete, erase, clear", "artboard, canvas");
        let problems = catalog_problems([(remove, "retrait"), (artboard, "plan, Retrait")]);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems.iter().all(|p| p.contains("retrait") && p.contains(remove) && p.contains(artboard)), "{problems:?}");
        assert!(catalog_problems([(remove, "retrait, retrait"), (remove, "Retrait"), (artboard, "plan")]).is_empty());
        assert!(catalog_problems([]).is_empty());
    }
}
