//! The keys of a text document written by name.
//!
//! Files saved for version 11 write every key of the text document by its name
//! (`/DocumentResources << /FontSet << /Resources [ ...`) and a typed node's kind under
//! `/StreamTag`, with the long names of the layout's strikes (`/FrameStrike`, `/LineStrike`). Later
//! versions write each key as its number (`/0 << /1 << /0 [ ...`, the kind under `/99`, `/F`,
//! `/L`), except the keys that are newer than the version the file is saved for, which they still
//! write by name (`/ParagraphDirection`, `/VersionString`). A key's number depends on the
//! dictionary it is in (`Leading` is 5 in a character style and 11 in a line), so the names are
//! read with the dictionary each one sits in, from the root down.
//!
//! A public reader of the format lists the names and numbers of the document's resources and
//! objects, the character and paragraph features, a story's model and view, a frame's data, the
//! line and glyph strikes, a font record's name, type and axes, and a color's type and values:
//! <https://github.com/Zignixx/firepixel/blob/60e3d5517df3106674b3208eb71e65471b4daec4/crates/fp-fwpng/src/ate_keys.rs>.
//! The other names here (the sets' lists, a resource and its kind, a font record's version, the
//! styles' name, parent and features, a Kinsoku set's name and data, a frame's point, curve, first
//! baseline and path options, the runs and their kerning, a strike's kind and origin, the tab stops
//! and the paints) are VectorCraft's own reading of the format, matched to the numbers the reader
//! already uses.
//!
//! Only the keys the text document's reader looks at, and the dictionaries on the way to them, are
//! given numbers; any other key keeps its name and is skipped as before. A key that the dictionary
//! also has as a number keeps the number's value.

use std::collections::BTreeMap;

use super::Val;

/// The kind of a dictionary of the text document, which gives its keys' numbers.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Root,
    Resources,
    Objects,
    FontSet,
    FontEntry,
    Font,
    FontRecord,
    KinsokuSet,
    KinsokuEntry,
    Kinsoku,
    CharSheetSet,
    CharSheetEntry,
    CharSheet,
    ParaSheetSet,
    ParaSheetEntry,
    ParaSheet,
    FrameSet,
    FrameEntry,
    Frame,
    Bezier,
    FrameData,
    FirstBaseline,
    PathOptions,
    TextObject,
    Model,
    ParaRuns,
    ParaRun,
    ParaRunData,
    CharRuns,
    CharRun,
    CharRunData,
    KernRuns,
    KernRun,
    KernRunData,
    View,
    FrameRef,
    Strike,
    Transform,
    CharFeatures,
    ParaFeatures,
    TabStops,
    TabStop,
    Paint,
    Color,
    /// A dictionary none of whose keys are read: its keys stay as they are.
    Other,
}

/// A key of a dictionary: its name, its number, and the kind of the dictionaries its value holds
/// (the value itself, or the elements of a list).
type Key = (&'static str, &'static str, Kind);

/// The keys of a set of resources (`FontSet`, `StyleSheetSet`, ...), whose list holds `entry`.
const fn set(entry: Kind) -> [Key; 2] {
    [("Resources", "0", entry), ("DisplayList", "1", Kind::Other)]
}

const FONT_SET: [Key; 2] = set(Kind::FontEntry);
const KINSOKU_SET: [Key; 2] = set(Kind::KinsokuEntry);
const CHAR_SHEET_SET: [Key; 2] = set(Kind::CharSheetEntry);
const PARA_SHEET_SET: [Key; 2] = set(Kind::ParaSheetEntry);
const FRAME_SET: [Key; 2] = set(Kind::FrameEntry);

/// The character features the reader looks at.
const CHAR_FEATURES: [Key; 20] = [
    ("Font", "0", Kind::Other),
    ("FontSize", "1", Kind::Other),
    ("AutoLeading", "4", Kind::Other),
    ("Leading", "5", Kind::Other),
    ("HorizontalScale", "6", Kind::Other),
    ("VerticalScale", "7", Kind::Other),
    ("Tracking", "8", Kind::Other),
    ("BaselineShift", "9", Kind::Other),
    ("FontCaps", "12", Kind::Other),
    ("BaselineDirection", "35", Kind::Other),
    ("LeftAki", "49", Kind::Other),
    ("RightAki", "50", Kind::Other),
    ("FillColor", "53", Kind::Paint),
    ("StrokeColor", "54", Kind::Paint),
    ("FillFlag", "56", Kind::Other),
    ("StrokeFlag", "57", Kind::Other),
    ("LineCap", "61", Kind::Other),
    ("LineJoin", "62", Kind::Other),
    ("LineWidth", "63", Kind::Other),
    ("MiterLimit", "64", Kind::Other),
];

/// The paragraph features the reader looks at.
const PARA_FEATURES: [Key; 22] = [
    ("Justification", "0", Kind::Other),
    ("FirstLineIndent", "1", Kind::Other),
    ("StartIndent", "2", Kind::Other),
    ("EndIndent", "3", Kind::Other),
    ("SpaceBefore", "4", Kind::Other),
    ("SpaceAfter", "5", Kind::Other),
    ("AutoLeading", "7", Kind::Other),
    ("AutoHyphenate", "9", Kind::Other),
    ("HyphenatedWordSize", "10", Kind::Other),
    ("PreHyphen", "11", Kind::Other),
    ("PostHyphen", "12", Kind::Other),
    ("ConsecutiveHyphens", "13", Kind::Other),
    ("Zone", "14", Kind::Other),
    ("HyphenateCapitalized", "15", Kind::Other),
    ("KeepTogether", "23", Kind::Other),
    ("BurasagariType", "24", Kind::Other),
    ("KinsokuOrder", "25", Kind::Other),
    ("Kinsoku", "27", Kind::Other),
    ("TabStops", "30", Kind::TabStops),
    ("DefaultTabWidth", "31", Kind::Other),
    ("DefaultStyle", "32", Kind::Other),
    ("ParagraphDirection", "33", Kind::Other),
];

/// The keys of a strike of the layout. A glyph strike has no children; its own keys start at 5. Its
/// glyph data (key 21) has no name here: the public list above does not name it, and the files
/// checked do not write it by name.
const STRIKE: [Key; 5] = [
    ("StreamTag", "99", Kind::Other),
    ("Transform", "0", Kind::Transform),
    ("Bounds", "1", Kind::Other),
    ("ChildProcession", "5", Kind::Other),
    ("Children", "6", Kind::Strike),
];
const GLYPH_STRIKE: [Key; 3] = [("StreamTag", "99", Kind::Other), ("Transform", "0", Kind::Transform), ("Bounds", "1", Kind::Other)];

/// The long names of the strikes' kinds, with the short ones later versions write.
const STRIKE_KINDS: [(&str, &str); 6] =
    [("PathSelectGroupCharacter", "PC"), ("FrameStrike", "F"), ("RowColStrike", "R"), ("LineStrike", "L"), ("Segment", "S"), ("GlyphStrike", "G")];

impl Kind {
    /// The keys of a dictionary of this kind; `tag` is the kind of typed node it is, for a strike.
    fn keys(self, tag: Option<&str>) -> &'static [Key] {
        use Kind as K;
        match self {
            K::Root => &[("DocumentResources", "0", K::Resources), ("DocumentObjects", "1", K::Objects)],
            K::Resources => &[
                ("FontSet", "1", K::FontSet),
                ("KinsokuSet", "4", K::KinsokuSet),
                ("StyleSheetSet", "5", K::CharSheetSet),
                ("ParagraphSheetSet", "6", K::ParaSheetSet),
                ("TextFrameSet", "8", K::FrameSet),
            ],
            K::Objects => &[
                ("DocumentSettings", "0", K::Other),
                ("TextObjects", "1", K::TextObject),
                ("OriginalNormalStyleFeatures", "2", K::CharFeatures),
                ("OriginalNormalParagraphFeatures", "3", K::ParaFeatures),
            ],
            K::FontSet => &FONT_SET,
            K::FontEntry => &[("Resource", "0", K::Font)],
            K::Font => &[("StreamTag", "99", K::Other), ("Identifier", "0", K::FontRecord)],
            K::FontRecord => &[("Name", "0", K::Other), ("Type", "2", K::Other), ("MMAxis", "4", K::Other), ("VersionString", "5", K::Other)],
            K::KinsokuSet => &KINSOKU_SET,
            K::KinsokuEntry => &[("Resource", "0", K::Kinsoku)],
            K::Kinsoku => &[("Name", "0", K::Other), ("Data", "5", K::Other)],
            K::CharSheetSet => &CHAR_SHEET_SET,
            K::CharSheetEntry => &[("Resource", "0", K::CharSheet)],
            K::CharSheet => &[("Name", "0", K::Other), ("Parent", "5", K::Other), ("Features", "6", K::CharFeatures)],
            K::ParaSheetSet => &PARA_SHEET_SET,
            K::ParaSheetEntry => &[("Resource", "0", K::ParaSheet)],
            K::ParaSheet => &[("Name", "0", K::Other), ("Features", "5", K::ParaFeatures), ("Parent", "6", K::Other)],
            K::FrameSet => &FRAME_SET,
            K::FrameEntry => &[("Resource", "0", K::Frame)],
            K::Frame => &[("Point", "0", K::Other), ("Bezier", "1", K::Bezier), ("Data", "2", K::FrameData)],
            K::Bezier => &[("Points", "0", K::Other)],
            K::FrameData => &[
                ("Type", "0", K::Other),
                ("LineOrientation", "1", K::Other),
                ("FrameMatrix", "2", K::Other),
                ("RowCount", "3", K::Other),
                ("ColumnCount", "4", K::Other),
                ("TextOnPathTRange", "6", K::Other),
                ("RowGutter", "7", K::Other),
                ("ColumnGutter", "8", K::Other),
                ("FirstBaselineAlignment", "10", K::FirstBaseline),
                ("PathData", "11", K::PathOptions),
            ],
            K::FirstBaseline => &[("Flag", "0", K::Other), ("Min", "1", K::Other)],
            K::PathOptions => &[("Flip", "0", K::Other), ("Effect", "1", K::Other), ("Alignment", "2", K::Other), ("CS3Behavior", "6", K::Other)],
            K::TextObject => &[("Model", "0", K::Model), ("View", "1", K::View)],
            K::Model => &[("Text", "0", K::Other), ("ParagraphRun", "5", K::ParaRuns), ("StyleRun", "6", K::CharRuns), ("KernRun", "8", K::KernRuns)],
            K::ParaRuns => &[("RunArray", "0", K::ParaRun)],
            K::ParaRun => &[("RunData", "0", K::ParaRunData), ("Length", "1", K::Other)],
            K::ParaRunData => &[("ParagraphSheet", "0", K::ParaSheet)],
            K::CharRuns => &[("RunArray", "0", K::CharRun)],
            K::CharRun => &[("RunData", "0", K::CharRunData), ("Length", "1", K::Other)],
            K::CharRunData => &[("StyleSheet", "0", K::CharSheet)],
            K::KernRuns => &[("RunArray", "0", K::KernRun)],
            K::KernRun => &[("RunData", "0", K::KernRunData), ("Length", "1", K::Other)],
            K::KernRunData => &[("ManualKern", "0", K::Other)],
            K::View => &[("Frames", "0", K::FrameRef), ("Strikes", "2", K::Strike)],
            K::FrameRef => &[("Resource", "0", K::Other)],
            K::Strike if matches!(tag, Some("GlyphStrike" | "G")) => &GLYPH_STRIKE,
            K::Strike => &STRIKE,
            K::Transform => &[("Origin", "0", K::Other)],
            K::CharFeatures => &CHAR_FEATURES,
            K::ParaFeatures => &PARA_FEATURES,
            K::TabStops => &[("TabStops", "0", K::TabStop)],
            K::TabStop => &[("TabAdvance", "0", K::Other), ("TabType", "1", K::Other)],
            K::Paint => &[("StreamTag", "99", K::Other), ("Color", "0", K::Color), ("CAIKnownStyleID", "5", K::Other)],
            K::Color => &[("Type", "0", K::Other), ("Values", "1", K::Other)],
            K::Other => &[],
        }
    }
}

/// The text document `doc` (its top-level keys) with the keys it writes by name given their
/// numbers, as later versions write them.
pub(super) fn numbered(doc: Val) -> Val {
    value(doc, Kind::Root)
}

/// `v`, whose dictionaries are of kind `kind`, with its keys numbered.
fn value(v: Val, kind: Kind) -> Val {
    match v {
        Val::Dict(d) if kind != Kind::Other => Val::Dict(dict(d, kind)),
        Val::List(l) if kind != Kind::Other => Val::List(l.into_iter().map(|x| value(x, kind)).collect()),
        v => v,
    }
}

/// A run's style written by name as the number `n` of a paragraph style (`kind` `ParaSheet`) or a
/// character style (`CharSheet`) of the document: a style with nothing of its own, based on that
/// one.
fn based_on(kind: Kind, n: f64) -> Option<Val> {
    let parent = match kind {
        Kind::ParaSheet => "6",
        Kind::CharSheet => "5",
        _ => return None,
    };
    Some(Val::Dict(BTreeMap::from([(parent.to_string(), Val::Num(n))])))
}

/// The entries of a dictionary of kind `kind` with their keys numbered.
fn dict(d: BTreeMap<String, Val>, kind: Kind) -> BTreeMap<String, Val> {
    let tag = match d.get("StreamTag").or_else(|| d.get("99")) {
        Some(Val::Name(n)) => Some(n.clone()),
        _ => None,
    };
    let keys = kind.keys(tag.as_deref());
    let mut out = BTreeMap::new();
    let mut named = vec![];
    for (k, v) in d {
        match keys.iter().find(|(name, number, _)| *name == k || *number == k) {
            Some((name, number, child)) => {
                let v = match (*number, v) {
                    ("99", Val::Name(n)) => Val::Name(STRIKE_KINDS.iter().find(|(long, _)| *long == n).map_or(n, |(_, short)| short.to_string())),
                    (_, Val::Num(n)) if k == *name => based_on(*child, n).unwrap_or(Val::Num(n)),
                    (_, v) => value(v, *child),
                };
                if k == *name {
                    named.push((number.to_string(), v));
                } else {
                    out.insert(k, v);
                }
            }
            None => {
                out.insert(k, v);
            }
        }
    }
    for (k, v) in named {
        out.entry(k).or_insert(v);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::read;
    use super::*;

    fn parse(s: &str) -> Val {
        read(s.as_bytes()).unwrap_or_else(|| panic!("{s}"))
    }

    #[test]
    fn named_keys_take_the_numbers_of_the_dictionary_they_are_in() {
        // `Leading` is 5 in a character style and 11 in a line; `Name` is 0 in a style and in a font
        // record.
        let named = parse(
            "/DocumentResources << /FontSet << /Resources [ << /Resource << /StreamTag /CoolTypeFont /Identifier << /Name (Serif) \
             /VersionString (Version 2) >> >> >> ] >> /StyleSheetSet << /Resources [ << /Resource << /Name (Red) /Parent 0 \
             /Features << /Leading 18.0 /FillColor << /StreamTag /SimplePaint /Color << /Type 1 /Values [ 1.0 1.0 0.0 0.0 ] >> >> >> >> >> ] >> >> \
             /DocumentObjects << /TextObjects [ << /Model << /Text (Hi) /ParagraphRun << /RunArray [ << /RunData << /ParagraphSheet 1 >> /Length 2 >> ] >> \
             /StyleRun << /RunArray [ << /RunData << /StyleSheet << /Parent 0 /Features << /FontSize 9.0 >> >> >> /Length 2 >> ] >> >> \
             /View << /Frames [ << /Resource 0 >> ] /Strikes [ << /StreamTag /FrameStrike /Transform << /Origin [ 1.0 2.0 ] >> \
             /Children [ << /StreamTag /LineStrike /Leading 14.4 /Children [ << /StreamTag /GlyphStrike /Glyphs [ 1 2 ] >> ] >> ] >> ] >> >> ] >>",
        );
        let numbered_form = parse(
            "/0 << /1 << /0 [ << /0 << /99 /CoolTypeFont /0 << /0 (Serif) /5 (Version 2) >> >> >> ] >> /5 << /0 [ << /0 << /0 (Red) /5 0 \
             /6 << /5 18.0 /53 << /99 /SimplePaint /0 << /0 1 /1 [ 1.0 1.0 0.0 0.0 ] >> >> >> >> >> ] >> >> \
             /1 << /1 [ << /0 << /0 (Hi) /5 << /0 [ << /0 << /0 << /6 1 >> >> /1 2 >> ] >> \
             /6 << /0 [ << /0 << /0 << /5 0 /6 << /1 9.0 >> >> >> /1 2 >> ] >> >> \
             /1 << /0 [ << /0 0 >> ] /2 [ << /99 /F /0 << /0 [ 1.0 2.0 ] >> \
             /6 [ << /99 /L /Leading 14.4 /6 [ << /99 /G /Glyphs [ 1 2 ] >> ] >> ] >> ] >> >> ] >>",
        );
        // A character style's `Leading` is its key 5; a line's and a glyph's keys the reader doesn't
        // look at keep their names.
        assert_eq!(numbered(named), numbered_form);
    }

    #[test]
    fn a_number_wins_over_the_same_key_by_name() {
        let doc = numbered(parse("/1 << /3 << /33 1 /ParagraphDirection 0 /Justification 2 >> >>"));
        assert_eq!(doc, parse("/1 << /3 << /33 1 /0 2 >> >>"));
    }

    #[test]
    fn keys_of_dictionaries_no_reader_looks_at_keep_their_names() {
        let doc = numbered(parse("/DocumentObjects << /DocumentSettings << /Name (x) /Leading 3 >> /Elsewhere << /Name (y) >> >>"));
        assert_eq!(doc, parse("/1 << /0 << /Name (x) /Leading 3 >> /Elsewhere << /Name (y) >> >>"));
    }

    #[test]
    fn a_numbered_document_stays_as_it_is() {
        let s = "/0 << /1 << /0 [ << /0 << /99 /CoolTypeFont /0 << /0 (Serif) >> >> >> ] >> >> \
                 /1 << /1 [ << /0 << /0 (Hi) /5 << /0 [ << /0 << /0 1 >> /1 2 >> ] >> /6 << /0 [ << /0 << /0 0 >> /1 2 >> ] >> >> \
                 /1 << /0 [ << /0 0 >> ] /2 [ << /99 /PC /6 [ << /99 /F /0 << /0 [ 1.0 2.0 ] >> >> ] >> ] >> >> ] /2 << /1 12.0 >> >>";
        // A run's style written as a number under its key's number stays a number.
        assert_eq!(numbered(parse(s)), parse(s));
    }
}
