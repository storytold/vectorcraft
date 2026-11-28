//! Liang pattern matching for the bundled hyph-utf8 Indic patterns. Provenance and licenses:
//! `crates/text/patterns/README.md`. Only bundled, trusted pattern data is parsed.
use std::collections::BTreeMap;
use std::sync::OnceLock;
use unicode_segmentation::UnicodeSegmentation;

pub(super) const TAGS: [&str; 11] = ["hi", "mr", "gu", "bn", "pa", "ta", "te", "kn", "ml", "or", "as"];
const SOURCES: [&str; 11] = [
    include_str!("../../patterns/hyph-hi.tex"),
    include_str!("../../patterns/hyph-mr.tex"),
    include_str!("../../patterns/hyph-gu.tex"),
    include_str!("../../patterns/hyph-bn.tex"),
    include_str!("../../patterns/hyph-pa.tex"),
    include_str!("../../patterns/hyph-ta.tex"),
    include_str!("../../patterns/hyph-te.tex"),
    include_str!("../../patterns/hyph-kn.tex"),
    include_str!("../../patterns/hyph-ml.tex"),
    include_str!("../../patterns/hyph-or.tex"),
    include_str!("../../patterns/hyph-as.tex"),
];

#[derive(Default)]
struct Node {
    next: BTreeMap<char, Node>,
    weights: Vec<u8>,
}

impl Node {
    /// Read only the patterns block of a trusted bundled TeX file, ignoring comments and macros.
    fn parse(source: &str) -> Self {
        let mut root = Self::default();
        let mut active = false;
        for line in source.lines() {
            let line = line.split('%').next().unwrap_or_default().trim();
            if line == "\\patterns{" {
                active = true;
                continue;
            }
            if line == "}" {
                active = false;
            }
            if !active || line.is_empty() {
                continue;
            }
            for pattern in line.split_whitespace() {
                let mut node = &mut root;
                let mut weights = vec![0];
                for ch in pattern.chars() {
                    if let Some(digit) = ch.to_digit(10) {
                        if let Some(last) = weights.last_mut() {
                            *last = digit as u8;
                        }
                    } else {
                        node = node.next.entry(ch).or_default();
                        weights.push(0);
                    }
                }
                node.weights = weights;
            }
        }
        root
    }

    /// Overlay matched Liang weights on a boundary-padded word; odd maxima permit breaks.
    fn points(&self, word: &str) -> Vec<usize> {
        let chars: Vec<char> = std::iter::once('.').chain(word.chars()).chain(std::iter::once('.')).collect();
        let mut weights = vec![0; chars.len() + 1];
        for start in 0..chars.len() {
            let mut node = self;
            for ch in chars.iter().skip(start) {
                let Some(next) = node.next.get(ch) else { break };
                node = next;
                for (slot, weight) in weights.iter_mut().skip(start).zip(&node.weights) {
                    *slot = (*slot).max(*weight);
                }
            }
        }
        (1..word.chars().count()).filter(|p| weights.get(p + 1).is_some_and(|w| w % 2 == 1)).collect()
    }
}

/// Lazily initialize the bundled tries and return character-index candidates for one language.
pub(super) fn points(word: &str, index: usize) -> Vec<usize> {
    static PATTERNS: OnceLock<Vec<Node>> = OnceLock::new();
    PATTERNS.get_or_init(|| SOURCES.iter().map(|s| Node::parse(s)).collect()).get(index).map_or_else(Vec::new, |p| p.points(word))
}

/// Patterns and user exceptions must both preserve a whole Unicode grapheme. This also protects
/// text when a fallback font subdivides a conjunct into several shaping clusters.
pub(super) fn retain_boundaries(word: &str, points: &mut Vec<usize>) {
    let mut count = 0;
    let mut boundaries = Vec::new();
    for cluster in word.graphemes(true) {
        boundaries.push(count);
        count += cluster.chars().count();
    }
    let chars: Vec<_> = word.chars().collect();
    points.retain(|p| {
        *p > 0
            && *p < count
            && boundaries.binary_search(p).is_ok()
            && !chars.get(*p).is_some_and(|c| matches!(c, '\u{200c}' | '\u{200d}'))
            && !p.checked_sub(1).and_then(|i| chars.get(i)).is_some_and(|c| {
                // Tamil's pulli is not a Unicode grapheme linker, and its patterns allow
                // coda breaks after it. Conservatively keep every virama with what follows,
                // even when neither the font nor Unicode groups the consonants together.
                matches!(
                    c,
                    '\u{200c}'
                        | '\u{200d}'
                        | '\u{094d}'
                        | '\u{09cd}'
                        | '\u{0a4d}'
                        | '\u{0acd}'
                        | '\u{0b4d}'
                        | '\u{0bcd}'
                        | '\u{0c4d}'
                        | '\u{0ccd}'
                        | '\u{0d4d}'
                )
            })
    });
}
