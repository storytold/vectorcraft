//! The text of an Illustrator editing copy.
//!
//! The editing copy of a file keeps the characters of its text objects, with their fonts, sizes,
//! colours and where they sit, in one text document of its own: between `%AI11_BeginTextDocument`
//! and `%AI11_EndTextDocument`, ASCII85 of nested dictionaries with numbers for keys. A text object
//! in a layer is only a stub naming its story in it (`/StoryIndex`).
//!
//! The format isn't documented. Most of what is read here was worked out from the files of the
//! project's users, used locally and never committed, by comparing it with the type their pages
//! draw: the meaning of those keys was checked against the page's own type in more than a thousand
//! text objects, and it is only used for the keys that matched. The Type on a Path Options of key
//! 11 of a frame come from test files saved with one option changed at a time, used locally and
//! never committed. Point type, area type and type on a path are read; a story in a frame of any
//! other kind is left alone: it has no text, and the importer says so.
//!
//! Where type sits: a story lays its lines out round a point `F` (the anchor: the start of a line of
//! left-aligned type, the middle of a centred one, the end of a right-aligned one), each line with an
//! offset from it. A right-to-left paragraph (key `33` of its attributes) swaps the two: its
//! "left"-aligned lines end at the anchor and its "right"-aligned ones start there. The frame of the
//! story gives a matrix that carries that layout onto the canvas of the app, which is 16383 points a
//! side; the canvas's centre, 8191.5, is the centre of the file's `%AI3_TemplateBox`. A story without
//! a layout has its point type's anchor at its frame's point. See [`Story::place`].

use std::collections::{BTreeMap, HashMap};

use kurbo::{BezPath, CubicBez, ParamCurve, ParamCurveArclen};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{
    CharStyle, HyphenationOptions, JustificationOptions, Justify, LineCap, LineJoin, Node, NodeId, NodeKind, ParaDirection, ParaStyle, PathAlign,
    PathEffect, TabAlign, TabStop, TextKind, TextObject, TextRun,
};
use vectorcraft_geom::{Affine, PathData, Point, Rect};

/// Longest story read (bytes).
pub(crate) const MAX_STORY: usize = 1 << 20;
/// Biggest text document read (bytes, decoded).
pub(crate) const MAX_BYTES: usize = 64 << 20;
/// Deepest nesting read.
const MAX_DEPTH: usize = 64;
/// Most values read.
pub(crate) const MAX_VALUES: usize = 4_000_000;
/// The centre of the app's canvas, in the units of its text document.
pub(super) const CANVAS_CENTRE: f64 = 8191.5;

/// A value of the text document.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Val {
    Num(f64),
    Str(String),
    Name(String),
    Bool(bool),
    List(Vec<Val>),
    Dict(BTreeMap<String, Val>),
}

impl Val {
    fn get(&self, key: &str) -> Option<&Val> {
        match self {
            Val::Dict(d) => d.get(key),
            _ => None,
        }
    }

    fn num(&self) -> Option<f64> {
        match self {
            Val::Num(n) if n.is_finite() => Some(*n),
            _ => None,
        }
    }

    fn str(&self) -> Option<&str> {
        match self {
            Val::Str(s) => Some(s),
            _ => None,
        }
    }

    fn list(&self) -> &[Val] {
        match self {
            Val::List(l) => l,
            _ => &[],
        }
    }

    fn nums(&self) -> Option<Vec<f64>> {
        self.list().iter().map(Val::num).collect()
    }

    /// The `name` entry of a dictionary that is a typed node (`/99 /name`).
    fn is_node(&self, name: &str) -> bool {
        matches!(self.get("99"), Some(Val::Name(n)) if n == name)
    }
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
    budget: usize,
}

impl Reader<'_> {
    fn skip_space(&mut self) {
        while self.b.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }

    fn value(&mut self, depth: usize) -> Option<Val> {
        self.skip_space();
        self.budget = self.budget.checked_sub(1)?;
        let c = *self.b.get(self.at)?;
        match c {
            b'<' if self.b.get(self.at + 1) == Some(&b'<') => {
                if depth > MAX_DEPTH {
                    return None;
                }
                self.at += 2;
                let mut d = BTreeMap::new();
                loop {
                    self.skip_space();
                    if self.b.get(self.at..self.at + 2) == Some(b">>") {
                        self.at += 2;
                        return Some(Val::Dict(d));
                    }
                    let key = self.key()?;
                    let v = self.value(depth + 1)?;
                    d.insert(key, v);
                }
            }
            b'[' => {
                if depth > MAX_DEPTH {
                    return None;
                }
                self.at += 1;
                let mut l = vec![];
                loop {
                    self.skip_space();
                    if self.b.get(self.at) == Some(&b']') {
                        self.at += 1;
                        return Some(Val::List(l));
                    }
                    l.push(self.value(depth + 1)?);
                }
            }
            b'(' => self.string(),
            b'/' => Some(Val::Name(self.key()?)),
            _ => {
                let start = self.at;
                while self.b.get(self.at).is_some_and(|c| !c.is_ascii_whitespace() && !matches!(c, b'<' | b'>' | b'[' | b']' | b'(' | b')' | b'/')) {
                    self.at += 1;
                }
                let word = std::str::from_utf8(self.b.get(start..self.at)?).ok()?;
                match word {
                    "true" => Some(Val::Bool(true)),
                    "false" => Some(Val::Bool(false)),
                    "" => None,
                    w => Some(Val::Num(w.parse().ok()?)),
                }
            }
        }
    }

    /// A `/name`, without its slash.
    fn key(&mut self) -> Option<String> {
        self.skip_space();
        if self.b.get(self.at) != Some(&b'/') {
            return None;
        }
        self.at += 1;
        let start = self.at;
        while self.b.get(self.at).is_some_and(|c| !c.is_ascii_whitespace() && !matches!(c, b'<' | b'>' | b'[' | b']' | b'(' | b')' | b'/')) {
            self.at += 1;
        }
        Some(String::from_utf8_lossy(self.b.get(start..self.at)?).into_owned())
    }

    /// A string: up to the first `)` that isn't escaped; UTF-16 when it starts with a byte order mark.
    fn string(&mut self) -> Option<Val> {
        self.at += 1;
        let mut bytes = vec![];
        loop {
            let c = *self.b.get(self.at)?;
            self.at += 1;
            match c {
                b')' => break,
                b'\\' => {
                    let e = *self.b.get(self.at)?;
                    self.at += 1;
                    bytes.push(match e {
                        b'n' => b'\n',
                        b'r' => b'\r',
                        b't' => b'\t',
                        b'b' => 8,
                        b'f' => 12,
                        b'0'..=b'7' => {
                            let mut v = u32::from(e - b'0');
                            for _ in 0..2 {
                                match self.b.get(self.at) {
                                    Some(d @ b'0'..=b'7') => {
                                        v = v * 8 + u32::from(d - b'0');
                                        self.at += 1;
                                    }
                                    _ => break,
                                }
                            }
                            (v & 0xff) as u8
                        }
                        other => other,
                    });
                }
                c => bytes.push(c),
            }
            if bytes.len() > MAX_BYTES {
                return None;
            }
        }
        Some(Val::Str(match bytes.strip_prefix(&[0xfe, 0xff]) {
            Some(rest) => {
                let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|p| u16::from_be_bytes(*p)).collect();
                String::from_utf16_lossy(&units)
            }
            None => bytes.iter().map(|b| char::from(*b)).collect(),
        }))
    }
}

/// The values of a text document: its top-level `/key value` pairs.
fn read(bytes: &[u8]) -> Option<Val> {
    let mut r = Reader { b: bytes, at: 0, budget: MAX_VALUES };
    let mut d = BTreeMap::new();
    loop {
        r.skip_space();
        if r.at >= bytes.len() {
            return Some(Val::Dict(d));
        }
        let key = r.key()?;
        let v = r.value(0)?;
        d.insert(key, v);
    }
}

/// Most character styles read of a text document, and most styles a run's style is followed
/// through to the ones they are based on.
const MAX_SHEETS: usize = 4096;
const MAX_SHEETS_BASED_ON: usize = 16;

/// Most frames read of one story, and most path segments of a frame.
pub(crate) const MAX_FRAMES: usize = 4096;
pub(crate) const MAX_SEGMENTS: usize = 1 << 16;

/// Where a frame puts its story's text.
#[derive(Debug, Clone, PartialEq)]
enum FrameKind {
    /// Point type: the matrix carries the layout (its anchor at the origin of its lines) onto the
    /// canvas.
    Point([f64; 6]),
    /// Area type flowed inside a path: its cubic segments on the canvas, four points each, and the
    /// Area Type Options the frame keeps.
    Area(Vec<[f64; 8]>, AreaFrame),
    /// Type on a path, from `start` to `end` (segment index and the place in that segment).
    OnPath { segments: Vec<[f64; 8]>, start: f64, end: f64, options: PathFrame },
}

/// The Type on a Path Options a frame of type on a path keeps at key 11. Each stays at its default
/// where the frame leaves the key out or keeps a value that can't be read. A frame with key 6 true
/// keeps its defaults: what that state stores for these options isn't known.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct PathFrame {
    /// Key 1: 1 Skew, 2 3D Ribbon, 3 Stair Step, 4 Gravity. Rainbow is the key left out.
    effect: PathEffect,
    /// Key 2: 0 Ascender, 1 Descender, 2 Center. Baseline is the key left out.
    align: PathAlign,
    /// Key 0: the type is flipped across its path. The frame's path runs as it did before the flip,
    /// from the start of its range.
    flip: bool,
}

impl PathFrame {
    /// The options frame data `carries` keeps.
    fn read(carries: &Val) -> PathFrame {
        let Some(options) = carries.get("11") else { return PathFrame::default() };
        if matches!(options.get("6"), Some(Val::Bool(true))) {
            return PathFrame::default();
        }
        let code = |k: &str| options.get(k).and_then(Val::num);
        let effect = match code("1") {
            Some(1.0) => PathEffect::Skew,
            Some(2.0) => PathEffect::Ribbon3d,
            Some(3.0) => PathEffect::StairStep,
            Some(4.0) => PathEffect::Gravity,
            _ => PathEffect::Rainbow,
        };
        let align = match code("2") {
            Some(0.0) => PathAlign::Ascender,
            Some(1.0) => PathAlign::Descender,
            Some(2.0) => PathAlign::Center,
            _ => PathAlign::Baseline,
        };
        PathFrame { effect, align, flip: matches!(options.get("0"), Some(Val::Bool(true))) }
    }
}

/// The Area Type Options an area frame keeps, in the units of the text document: each `None`
/// where the frame leaves the key out or keeps a value that can't be read, so the option stays as
/// it was.
#[derive(Debug, Clone, Default, PartialEq)]
struct AreaFrame {
    /// Key 3, the rows.
    rows: Option<usize>,
    /// Key 4, the columns.
    columns: Option<usize>,
    /// Key 7, the gutter between rows.
    row_gutter: Option<f64>,
    /// Key 8, the gutter between columns.
    column_gutter: Option<f64>,
    /// Key 10, the first baseline (`0` its kind, `1` its minimum). Without it the first baseline
    /// is Ascent.
    first_baseline: Option<(vectorcraft_doc::FirstBaseline, f64)>,
}

/// Longest gutter or first-baseline minimum, as `text.areaOptions` caps them: read in the units of
/// the text document, and again in document points once scaled.
const MAX_AREA_LENGTH: f64 = 10_000.0;

impl AreaFrame {
    /// The options frame data `carries` keeps.
    fn read(carries: &Val) -> AreaFrame {
        // A whole number, at most `max`.
        let whole = |v: &Val, max: f64| v.num().filter(|n| n.fract() == 0.0 && *n >= 0.0).map(|n| n.min(max) as usize);
        let max = vectorcraft_doc::AreaOptions::MAX_ROWS_COLUMNS;
        let count = |k: &str| carries.get(k).and_then(|v| whole(v, max as f64)).filter(|n| *n >= 1);
        let length = |v: Option<&Val>| v.and_then(Val::num).filter(|n| *n >= 0.0).map(|n| n.min(MAX_AREA_LENGTH));
        let first_baseline = carries.get("10").and_then(|b| {
            use vectorcraft_doc::FirstBaseline as F;
            let kind = match whole(b.get("0")?, 99.0)? {
                0 => F::Fixed,
                2 => F::CapHeight,
                3 => F::Leading,
                4 => F::XHeight,
                // Em Box Height (5) and Legacy (6) have no first baseline of their own here: the
                // first baseline stays as it was.
                _ => return None,
            };
            Some((kind, length(b.get("1")).unwrap_or(0.0)))
        });
        AreaFrame {
            rows: count("3"),
            columns: count("4"),
            row_gutter: length(carries.get("7")),
            column_gutter: length(carries.get("8")),
            first_baseline,
        }
    }

    /// Sets the options it keeps on `area`, its lengths at `scale` (document points to a unit).
    fn apply(&self, area: &mut vectorcraft_doc::AreaOptions, scale: f64) {
        if let Some(n) = self.rows {
            area.rows = n;
        }
        if let Some(n) = self.columns {
            area.columns = n;
        }
        // A gutter the frame leaves out stays as it was.
        let (row, column) = (area.row_gutter.unwrap_or(area.gutter), area.gutter);
        let scaled = |g: f64| (g * scale).min(MAX_AREA_LENGTH);
        let row = self.row_gutter.map_or(row, scaled);
        area.gutter = self.column_gutter.map_or(column, scaled);
        area.row_gutter = (row != area.gutter).then_some(row);
        if let Some((kind, min)) = self.first_baseline {
            area.first_baseline = kind;
            area.first_baseline_min = scaled(min);
        }
    }
}

/// A paragraph's attributes, as the text document keeps them.
#[derive(Clone, Debug, Default, PartialEq)]
struct Para {
    justify: u8,
    /// A right-to-left paragraph: its left and right alignments are each other's (the line of
    /// "left"-aligned type ends at its anchor).
    rtl: bool,
    first_indent: f64,
    left_indent: f64,
    right_indent: f64,
    space_before: f64,
    space_after: f64,
    hyphenate: bool,
    /// Key 7: the share of its type size a line takes as its auto leading (1.2: 120%, the default;
    /// Justification… sets 0 to 500%:
    /// https://helpx.adobe.com/illustrator/using/line-character-spacing.html). `None`: the default.
    auto_leading: Option<f64>,
    /// Keys 10 to 15, in the order of Paragraph panel menu › Hyphenation…: Words Longer Than,
    /// After First, Before Last, Hyphen Limit, Hyphenation Zone (pt) and Hyphenate Capitalized
    /// Words. A value that can't be read, or is out of its range, keeps the default.
    hyphenation: HyphenationOptions,
    /// Key 30, its list (key 0): the tab stops, each its position from the frame's left edge (key
    /// 0) and alignment (key 1: 0 left, the default, 1 center, 2 right, 3 decimal, in the order of
    /// the Tabs panel's buttons), sorted. A stop that can't be read is left out, and so are those
    /// past the first [`MAX_TABS`]. Leaders aren't read.
    tabs: Vec<TabStop>,
    /// The paragraph has more than [`MAX_TABS`] stops: those past them were left out.
    tabs_truncated: bool,
}

/// Most tab stops read for one paragraph.
const MAX_TABS: usize = 1024;
/// Largest tab stop position read (in the text document's units), as `text.tabs.set` takes it.
const MAX_TAB_POSITION: f64 = 100_000.0;
/// Most tab stops a story's paragraphs take in all: a paragraph style's stops are copied into
/// every paragraph that has them. The paragraphs past it take the default stops, with a warning.
const MAX_STORY_TABS: usize = 1 << 20;
/// The warning when [`MAX_STORY_TABS`] leaves tab stops out.
pub(crate) const TABS_LEFT_OUT: &str = "tab stops past VectorCraft's limits were left out";

/// The text of one story, in the units of the text document.
#[derive(Debug)]
pub(super) struct Story {
    /// The characters, with `\r` ending paragraphs and `\x03` ending lines.
    text: String,
    /// The anchor of the first frame's layout.
    anchor: (f64, f64),
    /// Where each of the first frame's lines starts, from the anchor.
    lines: Vec<(f64, f64)>,
    /// The frames the story flows through, in order.
    frames: Vec<FrameKind>,
    /// `(characters, paragraph)` in order.
    paras: Vec<(usize, Para)>,
    /// `(characters, style)` in order.
    runs: Vec<(usize, Style)>,
    /// Type on a path: the spacing its first frame's layout implies ([`path_spacing`]).
    path_spacing: f64,
    /// Vertical type (Type › Type Orientation › Vertical): its lines are columns, from right to
    /// left. Its layout's lines run down from the anchor, each column further left.
    vertical: bool,
    /// Tab stops past [`MAX_STORY_TABS`] were left out of its paragraphs.
    tabs_left_out: std::cell::Cell<bool>,
}

/// Spend `budget` on the tab stops `tabs`: when there aren't enough left, they're left out (the
/// default stops apply). Whether they were.
fn spend_tabs(tabs: &mut Vec<TabStop>, budget: &mut usize) -> bool {
    if tabs.len() <= *budget {
        *budget -= tabs.len();
        false
    } else {
        tabs.clear();
        true
    }
}

#[derive(Clone, Debug)]
struct Style {
    font: Option<String>,
    size: f64,
    /// `None`: auto.
    leading: Option<f64>,
    tracking: f64,
    baseline_shift: f64,
    h_scale: f64,
    v_scale: f64,
    fill: Option<Color>,
    stroke: Option<Color>,
    /// The pattern that paints the fill or the stroke in place of its colour: the one the art
    /// style named by key 5 of its paint gives ([`Texts::with_patterns`]), placed in the document.
    /// `None` for any other paint, which keeps its colour.
    fill_pattern: Option<Paint>,
    stroke_pattern: Option<Paint>,
    stroke_width: f64,
    /// Key 61, the stroke's cap: 0 butt, 1 round, 2 projecting (square).
    stroke_cap: LineCap,
    /// Key 62, the stroke's join: 0 miter, 1 round, 2 bevel.
    stroke_join: LineJoin,
    /// Key 64, the stroke's miter limit (a multiple of the stroke weight).
    stroke_miter_limit: f64,
    /// Key 12, the case: 1 Small Caps, 2 All Caps.
    caps: u8,
    /// Key 35 at 3: Tate-chu-yoko, the characters set across the column as one block.
    tate_chu_yoko: bool,
    /// Key 35 at 1 (Standard): Standard Vertical Roman Alignment, half-width characters upright in
    /// a column. Any other value, or no key in the run, its character style or the defaults, is off.
    standard_vertical_roman: bool,
}

/// The text document of an editing copy.
pub(super) struct Texts {
    stories: Vec<Val>,
    fonts: Vec<String>,
    /// The character and paragraph styles a story starts from.
    defaults: Val,
    para_defaults: Val,
    /// The document's character styles (`[Normal Character Style]` first), each a name, the
    /// character style it is based on (key 5, an index into these) and its attributes (key 6).
    char_sheets: Vec<Val>,
    /// The document's paragraph styles (`[Normal Paragraph Style]` first), each a name, its
    /// attributes (key 5) and the paragraph style it is based on (key 6, an index into these).
    para_sheets: Vec<Val>,
    frames: Vec<Val>,
    /// The patterns that paint characters, by the name of the art style that gives each
    /// ([`Self::with_patterns`]).
    patterns: HashMap<String, Paint>,
}

impl Texts {
    /// The text document with its characters painted by `patterns`: the patterns of the editing
    /// data's art styles that paint with one, each placed in the document, by the style's name.
    /// A run's fill or stroke (key 53 or 54) is a paint whose key 0 is a colour and whose key 5
    /// names the art style that paints it. In users' files (used locally, never committed), such a
    /// style is one paint: a colour, which key 0 already has, or a pattern, whose key 0 is black
    /// (`/0 0 /1 [ 1 0 ]`). The pattern's matrix is in art space, as a path's pattern fill has it:
    /// the files' own pages tile the type with the matrix a path's fill would get there.
    pub(super) fn with_patterns(self, patterns: HashMap<String, Paint>) -> Texts {
        Texts { patterns, ..self }
    }

    /// The text document in the editing copy `data`, if it has one that can be read.
    pub(super) fn read(data: &[u8]) -> Option<Texts> {
        fn find(h: &[u8], n: &[u8], from: usize) -> Option<usize> {
            h.get(from..)?.windows(n.len()).position(|w| w == n).map(|p| p + from)
        }
        let start = find(data, b"%AI11_BeginTextDocument", 0)?;
        let from = find(data, b"/ASCII85Decode", start)? + b"/ASCII85Decode".len();
        let end = find(data, b"~>", from)? + 2;
        // Each line after the first starts with a `%`.
        // ASCII85 makes at most four bytes of a character (`z`): more than this would be over `MAX_BYTES`.
        if end - from > MAX_BYTES / 4 {
            return None;
        }
        let packed: String = String::from_utf8_lossy(data.get(from..end)?)
            .split(['\r', '\n'])
            .enumerate()
            .map(|(i, l)| if i > 0 { l.strip_prefix('%').unwrap_or(l) } else { l })
            .collect::<String>()
            .trim_start_matches([' ', ','])
            .to_string();
        let bytes = crate::ps::ascii85_decode(&packed)?;
        if bytes.len() > MAX_BYTES {
            return None;
        }
        let doc = read(&bytes)?;
        let fonts = doc
            .get("0")?
            .get("1")?
            .get("0")?
            .list()
            .iter()
            .map(|f| f.get("0").and_then(|f| f.get("0")).and_then(|f| f.get("0")).and_then(Val::str).unwrap_or_default().to_string())
            .collect();
        let main = doc.get("1")?;
        let sheets = |key: &str| -> Vec<Val> {
            let list = doc.get("0").and_then(|d| d.get(key)).and_then(|s| s.get("0")).map(Val::list).unwrap_or_default();
            list.iter().take(MAX_SHEETS).map(|s| s.get("0").cloned().unwrap_or(Val::Dict(BTreeMap::new()))).collect()
        };
        Some(Texts {
            stories: main.get("1")?.list().to_vec(),
            fonts,
            defaults: main.get("2")?.clone(),
            para_defaults: main.get("3").cloned().unwrap_or(Val::Dict(BTreeMap::new())),
            char_sheets: sheets("5"),
            para_sheets: sheets("6"),
            frames: doc.get("0")?.get("8")?.get("0")?.list().to_vec(),
            patterns: HashMap::new(),
        })
    }

    /// Frame `i` of the document, if it is of a kind this reads.
    fn frame(&self, i: usize) -> Option<FrameKind> {
        let f = self.frames.get(i)?.get("0")?;
        let carries = f.get("2")?;
        let segments = || -> Option<Vec<[f64; 8]>> {
            let n = f.get("1")?.get("0")?.nums()?;
            let (chunks, rest) = n.as_chunks::<8>();
            (rest.is_empty() && !chunks.is_empty() && chunks.len() <= MAX_SEGMENTS).then(|| chunks.to_vec())
        };
        match carries.get("0").and_then(Val::num).unwrap_or(0.0) as i64 {
            // Point type that is neither turned nor scaled has no matrix (Illustrator 24 leaves it out):
            // its layout is already where it goes on the canvas.
            0 => Some(FrameKind::Point(match carries.get("2") {
                None => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                Some(m) => m.nums()?.try_into().ok()?,
            })),
            1 => Some(FrameKind::Area(segments()?, AreaFrame::read(carries))),
            2 => {
                let range = carries.get("6").and_then(Val::nums).unwrap_or_default();
                let segments = segments()?;
                let n = segments.len() as f64;
                let finite = |v: Option<&f64>| v.copied().filter(|v| v.is_finite());
                let (start, end) = if closed(&segments) {
                    // Round a closed path the end is written unwrapped (the start plus the run, so
                    // past the last segment), or wrapped the other way: both brackets are places on
                    // the loop.
                    let start = finite(range.first()).unwrap_or(0.0).rem_euclid(n);
                    (start, finite(range.get(1)).map_or(start, |e| e.rem_euclid(n)))
                } else {
                    let start = finite(range.first()).unwrap_or(0.0).clamp(0.0, n);
                    (start, finite(range.get(1)).unwrap_or(n).clamp(start, n))
                };
                Some(FrameKind::OnPath { segments, start, end, options: PathFrame::read(carries) })
            }
            _ => None,
        }
    }

    /// Is frame `i` vertical type? Key 1 of what it carries: 2 vertical, 0 or none horizontal. Any
    /// other value is ignored: horizontal.
    fn frame_vertical(&self, i: usize) -> bool {
        let orientation = self.frames.get(i).and_then(|f| f.get("0")).and_then(|f| f.get("2")).and_then(|c| c.get("1"));
        orientation.and_then(Val::num) == Some(2.0)
    }

    /// Story `i`, if its frames are of kinds this reads.
    pub(super) fn story(&self, i: usize) -> Option<Story> {
        let s = self.stories.get(i)?;
        let text = s.get("0")?.get("0")?.str().filter(|t| t.len() <= MAX_STORY)?.to_string();
        let view = s.get("1")?;
        let indices: Vec<usize> =
            view.get("0")?.list().iter().take(MAX_FRAMES).map(|f| f.get("0").and_then(Val::num).map(|n| n as usize)).collect::<Option<_>>()?;
        let frames: Vec<FrameKind> = indices.iter().map(|n| self.frame(*n)).collect::<Option<_>>()?;
        if frames.is_empty() {
            return None;
        }
        // Threaded frames share one orientation: frames that differ aren't read.
        let vertical = indices.first().is_some_and(|n| self.frame_vertical(*n));
        if indices.iter().any(|n| self.frame_vertical(*n) != vertical) {
            return None;
        }
        // The first frame's layout, when the file keeps one: where its lines start. Without one, the
        // anchor of point type is its frame's point (key 0, where the layout's anchor is too).
        let point = indices.first().and_then(|n| self.frames.get(*n)).and_then(|f| f.get("0")).and_then(|f| f.get("0")).and_then(Val::nums);
        let (mut anchor, mut lines, mut path_spacing) = (
            match point.as_deref() {
                Some([x, y]) if matches!(frames.first(), Some(FrameKind::Point(_))) => (*x, *y),
                _ => (0.0, 0.0),
            },
            vec![],
            0.0,
        );
        if let Some(f) = view.get("2").and_then(|l| find_node(l, "F")) {
            // Vertical type on a path turns its glyphs across the path: their matrices don't give
            // the spacing.
            if let Some(FrameKind::OnPath { options, .. }) = frames.first()
                && !vertical
            {
                path_spacing = self::path_spacing(f, options.effect);
            }
            if let Some(a) = f.get("0").and_then(|a| a.get("0")).and_then(Val::nums)
                && let [x, y, ..] = a.as_slice()
            {
                anchor = (*x, *y);
            }
            let mut found = vec![];
            find_nodes(f, "L", &mut found);
            for l in found {
                let off = l.get("0").and_then(|o| o.get("0")).and_then(Val::nums).unwrap_or_else(|| vec![0.0, 0.0]);
                let Some(seg) = l.get("6").map(Val::list).and_then(|c| c.iter().find(|c| c.is_node("S"))) else { continue };
                let so = seg.get("0").and_then(|o| o.get("0")).and_then(Val::nums).unwrap_or_else(|| vec![0.0, 0.0]);
                if let (Some(ox), Some(oy), Some(sx)) = (off.first(), off.get(1), so.first()) {
                    lines.push((ox + sx, *oy));
                }
            }
        }
        let mut paras = vec![];
        for run in s.get("0")?.get("5").map(|p| p.get("0").map(Val::list).unwrap_or_default()).unwrap_or_default() {
            let n = run.get("1").and_then(Val::num)? as usize;
            paras.push((n, self.para(run.get("0").and_then(|r| r.get("0")))));
        }
        if paras.is_empty() {
            paras.push((text.chars().count(), self.para(None)));
        }
        let mut runs = vec![];
        for run in s.get("0")?.get("6").map(|p| p.get("0").map(Val::list).unwrap_or_default()).unwrap_or_default() {
            let n = run.get("1").and_then(Val::num)? as usize;
            runs.push((n, self.style(run.get("0").and_then(|r| r.get("0")))));
        }
        if runs.is_empty() {
            runs.push((text.chars().count(), self.style(None)));
        }
        let mut budget = MAX_STORY_TABS;
        let mut tabs_left_out = false;
        for (_, p) in &mut paras {
            tabs_left_out |= p.tabs_truncated | spend_tabs(&mut p.tabs, &mut budget);
        }
        Some(Story { text, anchor, lines, frames, paras, runs, path_spacing, vertical, tabs_left_out: tabs_left_out.into() })
    }

    /// A paragraph's attributes (`run`: its attributes, key 5, and the paragraph style it is based
    /// on, key 6): its own keys, else those of its paragraph style and the ones that is based on,
    /// else those of `[Normal Paragraph Style]`, else the document's.
    fn para(&self, run: Option<&Val>) -> Para {
        let mut chain = vec![];
        let mut at = run;
        for _ in 0..=MAX_SHEETS_BASED_ON {
            let Some(style) = at else { break };
            chain.extend(style.get("5"));
            at = style.get("6").and_then(Val::num).filter(|i| i.is_finite() && *i >= 0.0).and_then(|i| self.para_sheets.get(i as usize));
        }
        chain.extend(self.para_sheets.first().and_then(|s| s.get("5")));
        let pick = |k: &str| chain.iter().find_map(|s| s.get(k)).or_else(|| self.para_defaults.get(k));
        let num = |k: &str| pick(k).and_then(Val::num).filter(|v| v.abs() < 1e5).unwrap_or(0.0);
        Para {
            justify: pick("0").and_then(Val::num).map_or(0, |v| v.clamp(0.0, 6.0) as u8),
            rtl: pick("33").and_then(Val::num) == Some(1.0),
            first_indent: num("1"),
            left_indent: num("2"),
            right_indent: num("3"),
            space_before: num("4"),
            space_after: num("5"),
            hyphenate: !matches!(pick("9"), Some(Val::Bool(false))),
            auto_leading: pick("7").and_then(Val::num).filter(|v| v.is_finite() && *v > 0.0 && *v <= 5.0),
            hyphenation: {
                let d = HyphenationOptions::default();
                let count = |k: &str, lo: f64, hi: f64, or: u32| {
                    pick(k).and_then(Val::num).filter(|v| v.is_finite() && v.fract() == 0.0 && (lo..=hi).contains(v)).map_or(or, |v| v as u32)
                };
                HyphenationOptions {
                    words_longer_than: count("10", 1.0, 25.0, d.words_longer_than),
                    after_first: count("11", 1.0, 15.0, d.after_first),
                    before_last: count("12", 1.0, 15.0, d.before_last),
                    hyphen_limit: count("13", 0.0, 25.0, d.hyphen_limit),
                    zone: pick("14").and_then(Val::num).filter(|v| v.is_finite() && (0.0..=1296.0).contains(v)).unwrap_or(d.zone),
                    capitalized: match pick("15") {
                        Some(Val::Bool(b)) => *b,
                        _ => d.capitalized,
                    },
                }
            },
            tabs_truncated: pick("30").and_then(|t| t.get("0")).map(Val::list).is_some_and(|list| list.len() > MAX_TABS),
            tabs: {
                let list = pick("30").and_then(|t| t.get("0")).map(Val::list).unwrap_or_default();
                let mut tabs: Vec<TabStop> = list
                    .iter()
                    .take(MAX_TABS)
                    .filter_map(|t| {
                        let position = t.get("0").and_then(Val::num).filter(|p| (0.0..=MAX_TAB_POSITION).contains(p))?;
                        let align = match t.get("1").map(Val::num) {
                            None | Some(Some(0.0)) => TabAlign::Left,
                            Some(Some(1.0)) => TabAlign::Center,
                            Some(Some(2.0)) => TabAlign::Right,
                            Some(Some(3.0)) => TabAlign::Decimal,
                            _ => return None,
                        };
                        Some(TabStop { position, align, leader: String::new(), align_on: '.' })
                    })
                    .collect();
                tabs.sort_by(|a, b| a.position.total_cmp(&b.position));
                tabs.dedup_by(|a, b| (a.position - b.position).abs() < 1e-6);
                tabs
            },
        }
    }

    /// A run's character style (`run`: its attributes, key 6, and the character style it is based
    /// on, key 5): its own keys, else those of its character style and the ones that is based on,
    /// else those of `[Normal Character Style]`, else the document's.
    fn style(&self, run: Option<&Val>) -> Style {
        // The run's attributes, then its character style's, then the styles each is based on.
        let mut chain = vec![];
        let mut at = run;
        for _ in 0..=MAX_SHEETS_BASED_ON {
            let Some(style) = at else { break };
            chain.extend(style.get("6"));
            at = style.get("5").and_then(Val::num).filter(|i| i.is_finite() && *i >= 0.0).and_then(|i| self.char_sheets.get(i as usize));
        }
        chain.extend(self.char_sheets.first().and_then(|s| s.get("6")));
        let pick = |k: &str| chain.iter().find_map(|s| s.get(k)).or_else(|| self.defaults.get(k));
        let num = |k: &str| pick(k).and_then(Val::num);
        let on = |k: &str| !matches!(pick(k), Some(Val::Bool(false)));
        let font = num("0").and_then(|i| self.fonts.get(i as usize)).filter(|f| !f.is_empty()).cloned();
        let size = num("1").filter(|v| *v > 0.0 && *v < 1e5).unwrap_or(12.0);
        // Key 5 of a paint names the art style that paints it ([`Self::with_patterns`]).
        let pattern = |k: &str| pick(k).and_then(|p| p.get("5")).and_then(Val::str).and_then(|name| self.patterns.get(name)).cloned();
        Style {
            font,
            size,
            leading: if on("4") { None } else { num("5").filter(|v| *v > 0.0 && *v < 1e5) },
            tracking: num("8").filter(|v| v.abs() < 1e5).unwrap_or(0.0),
            baseline_shift: num("9").filter(|v| v.abs() < 1e5).unwrap_or(0.0),
            h_scale: num("6").filter(|v| *v > 0.0 && *v < 100.0).unwrap_or(1.0),
            v_scale: num("7").filter(|v| *v > 0.0 && *v < 100.0).unwrap_or(1.0),
            fill: if on("56") { Some(pick("53").and_then(paint).unwrap_or(Color::BLACK)) } else { None },
            stroke: if matches!(pick("57"), Some(Val::Bool(true))) { pick("54").and_then(paint) } else { None },
            fill_pattern: pattern("53"),
            stroke_pattern: pattern("54"),
            stroke_width: num("63").filter(|v| *v >= 0.0 && *v < 1e4).unwrap_or(1.0),
            // Numbered as PostScript's `setlinecap` and `setlinejoin` operators number them
            // (PostScript Language Reference, 3rd ed.): in users' files the page strokes type with
            // the cap and join these give, and with key 64 as its miter limit. No file had a bevel
            // join (2). Any other value keeps the default.
            stroke_cap: match num("61") {
                Some(1.0) => LineCap::Round,
                Some(2.0) => LineCap::Square,
                _ => LineCap::Butt,
            },
            stroke_join: match num("62") {
                Some(1.0) => LineJoin::Round,
                Some(2.0) => LineJoin::Bevel,
                _ => LineJoin::Miter,
            },
            stroke_miter_limit: num("64").filter(|v| v.is_finite() && *v >= 1.0).map_or(10.0, |v| v.min(500.0)),
            caps: num("12").filter(|v| *v == 1.0 || *v == 2.0).map_or(0, |v| v as u8),
            tate_chu_yoko: num("35") == Some(3.0),
            standard_vertical_roman: num("35") == Some(1.0),
        }
    }
}

/// The colour of a paint of the text document: kind 0 grey (`[alpha, level]`, 1 is white), 1 RGB
/// (`[alpha, r, g, b]`), 2 CMYK (`[alpha, c, m, y, k]`).
fn paint(p: &Val) -> Option<Color> {
    let p = p.get("0")?;
    let v = p.get("1")?.nums()?;
    let unit = |i: usize| v.get(i).map(|c| c.clamp(0.0, 1.0) as f32);
    match p.get("0")?.num()? as i64 {
        0 => Some(Color::gray(1.0 - unit(1)?)),
        1 => Some(Color::rgb(unit(1)?, unit(2)?, unit(3)?)),
        2 => Some(Color::cmyk(unit(1)?, unit(2)?, unit(3)?, unit(4)?)),
        _ => None,
    }
}

fn find_node<'a>(v: &'a Val, name: &str) -> Option<&'a Val> {
    let mut out = vec![];
    find_nodes(v, name, &mut out);
    out.into_iter().next()
}

/// The typed nodes called `name` in `v`, in order, outermost first.
fn find_nodes<'a>(v: &'a Val, name: &str, out: &mut Vec<&'a Val>) {
    match v {
        Val::Dict(d) => {
            if v.is_node(name) {
                out.push(v);
            }
            for x in d.values() {
                find_nodes(x, name, out);
            }
        }
        Val::List(l) => {
            for x in l {
                find_nodes(x, name, out);
            }
        }
        _ => {}
    }
}

/// A point of the canvas (y down) in the units of the art (y up), given the centre `template` of
/// the file's template box.
fn to_art((x, y): (f64, f64), template: (f64, f64)) -> (f64, f64) {
    (x - CANVAS_CENTRE + template.0, CANVAS_CENTRE + template.1 - y)
}

/// Segments of the canvas as a path of the document.
fn path_of(segments: &[[f64; 8]], template: (f64, f64), to_doc: Affine) -> PathData {
    let at = |x: f64, y: f64| {
        let (x, y) = to_art((x, y), template);
        to_doc * Point::new(x, y)
    };
    let mut bp = BezPath::new();
    for (i, [x0, y0, x1, y1, x2, y2, x3, y3]) in segments.iter().enumerate() {
        if i == 0 {
            bp.move_to(at(*x0, *y0));
        }
        bp.curve_to(at(*x1, *y1), at(*x2, *y2), at(*x3, *y3));
    }
    let closed = closed(segments);
    if closed {
        bp.close_path();
    }
    PathData::from_bezpath(&bp)
}

/// Do `segments` end where they start (a closed path)?
fn closed(segments: &[[f64; 8]]) -> bool {
    matches!((segments.first(), segments.last()), (Some(f), Some(l)) if (f[0] - l[6]).abs() < 1e-3 && (f[1] - l[7]).abs() < 1e-3)
}

/// Where on `segments` the place `t` (segment index and the place in it) is, as a fraction of their
/// length.
fn length_fraction(segments: &[[f64; 8]], t: f64) -> f64 {
    let cubic = |s: &[f64; 8]| CubicBez::new((s[0], s[1]), (s[2], s[3]), (s[4], s[5]), (s[6], s[7]));
    let lengths: Vec<f64> = segments.iter().map(|s| cubic(s).arclen(0.01)).collect();
    let total: f64 = lengths.iter().sum();
    if !total.is_finite() || total <= 1e-9 {
        return 0.0;
    }
    let whole = t.floor().max(0.0) as usize;
    let before: f64 = lengths.iter().take(whole).sum();
    let within = match segments.get(whole) {
        Some(s) => cubic(s).subsegment(0.0..t.fract().clamp(0.0, 1.0)).arclen(0.01),
        None => 0.0,
    };
    ((before + within) / total).clamp(0.0, 1.0)
}

/// The spacing (Type on a Path Options › Spacing) that the layout `f` of type on a path implies.
/// The file closes its glyphs up round the outside of a curve and opens them up round the inside,
/// even where its frame keeps no spacing; the text engine spaces glyphs that way by the spacing
/// (`vectorcraft_text`'s `path_steps`): between two glyphs' centres, the path's length `s`, their
/// advances' mean `a` and how far it turns `t` (away from their tops) make `s + t × spacing = a`.
/// Fitted over every pair of the layout's glyphs; none where the path doesn't turn under them.
/// The length `s` is on the path: Align to Path moves the baselines off it, by the distance `G`
/// keeps. Under 3D Ribbon the glyphs' baselines stay level, so their feet show how the path turns.
fn path_spacing(f: &Val, effect: PathEffect) -> f64 {
    let mut found = vec![];
    find_nodes(f, "G", &mut found);
    let (mut sum, mut turns) = (0.0, 0.0);
    for g in found {
        // How far the baseline lies from the path, towards the glyphs' feet. Only Rainbow's and 3D
        // Ribbon's feet are square to the path; Skew's are upright, so its alignment moves every
        // baseline the same way and leaves the distances between them.
        let offset = if matches!(effect, PathEffect::Rainbow | PathEffect::Ribbon3d) {
            g.get("0").and_then(|o| o.get("0")).and_then(Val::nums).and_then(|o| o.get(1).copied()).filter(|o| o.is_finite()).unwrap_or(0.0)
        } else {
            0.0
        };
        // Each glyph's origin, its matrix (along its baseline, then towards its foot) and where it
        // ends along the line; the lists end with where the line ends.
        let Some(at) = g.get("21") else { continue };
        let list = |k: &str| at.get(k).and_then(Val::nums).unwrap_or_default();
        let (xs, ys, matrices, ends) = (list("1"), list("2"), list("3"), list("4"));
        if effect == PathEffect::StairStep {
            let (s, t) = stair_step_fit(&xs, &ys, &ends);
            sum += s;
            turns += t;
            continue;
        }
        let glyphs: Vec<(Point, f64, kurbo::Vec2, kurbo::Vec2)> = xs
            .iter()
            .zip(&ys)
            .zip(matrices.as_chunks::<6>().0)
            .zip(ends.windows(2))
            .map(|(((x, y), [a, b, c, d, ..]), e)| {
                let advance = e.get(1).zip(e.first()).map_or(0.0, |(e1, e0)| e1 - e0);
                let along = kurbo::Vec2::new(*a, *b);
                (Point::new(*x, *y) + along * (advance * 0.5), advance, along, kurbo::Vec2::new(*c, *d))
            })
            .collect();
        for w in glyphs.windows(2) {
            let [(p, a0, along, foot), (q, a1, next, next_foot)] = w else { continue };
            // 3D Ribbon's level baseline faces the way the path runs, so where the path runs left its
            // baseline and foot are turned round: `side` turns them back.
            let side = |baseline: &kurbo::Vec2| if effect == PathEffect::Ribbon3d && baseline.x < 0.0 { -1.0 } else { 1.0 };
            let (side, next_side) = (side(along), side(next));
            let (along, next) = if effect == PathEffect::Ribbon3d {
                (kurbo::Vec2::new(foot.y, -foot.x) * side, kurbo::Vec2::new(next_foot.y, -next_foot.x) * next_side)
            } else {
                (*along, *next)
            };
            // The turn is measured against a foot square to the path: Rainbow's and 3D Ribbon's own,
            // else the baseline turned a right angle, as theirs is (Skew's feet stay upright).
            let square =
                if matches!(effect, PathEffect::Rainbow | PathEffect::Ribbon3d) { *foot * side } else { kurbo::Vec2::new(-along.y, along.x) };
            let turn = next.dot(square).atan2(next.dot(along));
            // The length of the arc between the centres (the path is taken to be round there). A
            // corner (a turn of more than a radian between two glyphs) isn't round: left out. So is a
            // 3D Ribbon pair either side of where the path turns back, when an alignment puts their
            // baselines on opposite sides of the path.
            let half = turn.abs() * 0.5;
            let chord = (*q - *p).hypot();
            let length = if half > 1e-9 { chord * half / half.sin() } else { chord } + turn * offset * side;
            let apart = side != next_side && offset != 0.0;
            if !apart && turn.abs() <= 1.0 && length.is_finite() && a0.is_finite() && a1.is_finite() {
                sum += ((a0 + a1) * 0.5 - length) * turn;
                turns += turn * turn;
            }
        }
    }
    let spacing = sum / turns;
    if turns > 1e-6 && spacing.is_finite() { spacing.clamp(-1000.0, 1000.0) } else { 0.0 }
}

/// [`path_spacing`]'s sums for one line of Stair Step type, from its glyphs' origins `xs`, `ys` and
/// where they end along the line, `ends`. Stair Step's glyphs stay upright, so their matrices don't
/// show how the path turns; each origin lies on the path (all of them moved the same way by an
/// alignment), and a glyph's step is the path's length from its origin to the next. The path is taken
/// to be round through three origins in a row, which gives how far it turns over that step `t`; the
/// text engine makes the step `s` with `s + t × spacing = a`, `a` the glyph's advance.
fn stair_step_fit(xs: &[f64], ys: &[f64], ends: &[f64]) -> (f64, f64) {
    let origins: Vec<Point> = xs.iter().zip(ys).map(|(x, y)| Point::new(*x, *y)).collect();
    let (mut sum, mut turns) = (0.0, 0.0);
    for i in 0..origins.len().saturating_sub(1) {
        let (Some(p), Some(q)) = (origins.get(i), origins.get(i + 1)) else { continue };
        // The third origin: the one before, else the one after.
        let triple = match i.checked_sub(1).and_then(|h| origins.get(h)) {
            Some(o) => (*o, *p, *q, *o),
            None => match origins.get(i + 2) {
                Some(r) => (*p, *q, *r, *r),
                None => continue,
            },
        };
        let (a, b, c, third) = triple;
        // The inscribed angle at the third origin is half the arc's angle from `p` to `q`.
        let (to_p, to_q) = ((*p - third).hypot(), (*q - third).hypot());
        let sine = (b - a).cross(c - a) / (to_p * to_q);
        if !(sine.is_finite() && sine.abs() < 1.0) {
            continue;
        }
        let turn = 2.0 * sine.asin();
        let half = turn.abs() * 0.5;
        let chord = (*q - *p).hypot();
        let length = if half > 1e-9 { chord * half / half.sin() } else { chord };
        let advance = ends.get(i + 1).zip(ends.get(i)).map_or(f64::NAN, |(e1, e0)| e1 - e0);
        if turn.abs() <= 1.0 && length.is_finite() && advance.is_finite() {
            sum += (advance - length) * turn;
            turns += turn * turn;
        }
    }
    (sum, turns)
}

impl Story {
    /// How many bytes of text the story has.
    pub(super) fn len(&self) -> usize {
        self.text.len()
    }

    /// The point frame's matrix (the first frame's, when it is one), else the identity.
    fn matrix(&self) -> [f64; 6] {
        match self.frames.first() {
            Some(FrameKind::Point(m)) => *m,
            _ => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        }
    }

    /// The anchor of the story on the file's canvas: `(x, y)` in the units of the art, y up, given
    /// the centre `(tx, ty)` of the file's template box.
    pub(super) fn place(&self, template: (f64, f64)) -> (f64, f64) {
        let [a, b, c, d, e, f] = self.matrix();
        to_art((a * self.anchor.0 + c * self.anchor.1 + e, b * self.anchor.0 + d * self.anchor.1 + f), template)
    }

    /// Each line's characters and where it starts (in the units of the art, y up): where the page
    /// draws the line when the type is shown.
    pub(super) fn line_starts(&self, template: (f64, f64)) -> Vec<(String, (f64, f64))> {
        let [a, b, c, d, e, f] = self.matrix();
        let text: Vec<&str> = self.text.split(['\r', '\x03']).collect();
        self.lines
            .iter()
            .enumerate()
            .map(|(i, (lx, ly))| {
                // A vertical layout's lines run down the canvas, and each next one is further left.
                let (x, y) = if self.vertical { (self.anchor.0 - ly, self.anchor.1 + lx) } else { (self.anchor.0 + lx, self.anchor.1 + ly) };
                (text.get(i).copied().unwrap_or_default().to_string(), to_art((a * x + c * y + e, b * x + d * y + f), template))
            })
            .collect()
    }

    /// Roughly where the story's type lies on the document: its frame's box, or round its lines
    /// for point type (what the page draws of it lies there).
    pub(super) fn region(&self, template: (f64, f64), to_doc: Affine) -> Option<Rect> {
        let size = self.runs.iter().map(|(_, s)| s.size).fold(0.0, f64::max) * art_scale(to_doc);
        let bounds = |segments: &[[f64; 8]]| path_of(segments, template, to_doc).bounds();
        let r = match self.frames.first()? {
            FrameKind::Area(s, _) => bounds(s)?,
            FrameKind::OnPath { segments, .. } => bounds(segments)?.inflate(size * 1.5, size * 1.5),
            FrameKind::Point(m) => {
                let longest = self.text.split(['\r', '\x03']).map(|l| l.chars().count()).max().unwrap_or(0) as f64;
                let reach = longest * size + size;
                let points: Vec<Point> = self.line_starts(template).into_iter().map(|(_, (x, y))| to_doc * Point::new(x, y)).collect();
                let first = points.first().copied().unwrap_or_else(|| {
                    let (x, y) = self.place(template);
                    to_doc * Point::new(x, y)
                });
                // Along the baseline as far as a line reaches, across it a line and a half: turned
                // with the type (a dimension's label turned a quarter runs down the page).
                // (The canvas is y down, the art y up: see [`to_art`].)
                let along = to_doc * Point::new(m[0], -m[1]) - to_doc * Point::ZERO;
                let along = if along.hypot() > 1e-9 && along.is_finite() { along / along.hypot() } else { kurbo::Vec2::new(1.0, 0.0) };
                let across = kurbo::Vec2::new(-along.y, along.x);
                // Vertical type: its lines run down the page, a quarter turn from the baseline.
                let (along, across) = if self.vertical { (across, along) } else { (along, across) };
                let mut r = Rect::from_points(first, first);
                for p in points.iter().chain([&first]) {
                    for (t, n) in [(-reach, -1.5), (-reach, 1.5), (reach, -1.5), (reach, 1.5)] {
                        r = r.union_pt(*p + along * t + across * (n * size));
                    }
                }
                r
            }
        };
        r.is_finite().then_some(r)
    }

    /// The story as the text object of its frame `frame` (`to_doc` takes the art's units onto the
    /// document), or `None` for type that is mirrored, has no size, or a frame past the first of
    /// threaded type (the first frame holds the whole story).
    pub(super) fn node(&self, id: NodeId, frame: usize, template: (f64, f64), to_doc: Affine) -> Option<Node> {
        let kind = self.frames.get(frame)?;
        let threaded = frame > 0;
        let (scale, xf, kind) = match kind {
            FrameKind::Point(m) => {
                let [a, b, c, d, ..] = *m;
                let scale = (a * d - b * c).abs().sqrt();
                if !(scale.is_finite() && scale > 1e-6 && a * d - b * c > 0.0) || threaded {
                    return None;
                }
                let (x, y) = self.place(template);
                let at = to_doc * Point::new(x, y);
                (scale, Affine::new([a / scale, b / scale, c / scale, d / scale, at.x, at.y]), TextKind::Point)
            }
            FrameKind::Area(s, _) => (1.0, Affine::IDENTITY, TextKind::Area { frame: path_of(s, template, to_doc) }),
            FrameKind::OnPath { segments, start, end, options } => {
                let n = segments.len() as f64;
                // A closed path keeps its end bracket wherever it is (the same place as the start: a
                // full turn, see `TextKind::path_span`); an open one's end at its end is none, unless
                // a flip makes it the place the type starts.
                let end = (closed(segments) || options.flip || *end < n - 0.02).then(|| length_fraction(segments, *end));
                (1.0, Affine::IDENTITY, TextKind::OnPath { path: path_of(segments, template, to_doc), start: length_fraction(segments, *start), end })
            }
        };
        // A `\x03` ends a line inside its paragraph: a forced line break (U+2028), which the
        // paragraph's style runs over.
        let text: String = if threaded {
            String::new()
        } else {
            // The `\r` that ends the story's last paragraph; one before it ends a paragraph, and the
            // empty paragraph after it is the story's last.
            self.text
                .strip_suffix('\r')
                .unwrap_or(&self.text)
                .chars()
                .map(|c| match c {
                    '\r' => '\n',
                    '\x03' => '\u{2028}',
                    c => c,
                })
                .collect()
        };
        if text.trim().is_empty() && !threaded {
            return None;
        }
        // The text document measures in the art's units too: the document's scale carries its
        // sizes and spacings onto the page (a Large Canvas copy is written at 1:10; #2031).
        let scale = scale * art_scale(to_doc);
        // Patterns stay where the document has them, wherever the type's text space is (`xf`'s
        // determinant is 1, so it can be inverted; a pattern that still doesn't map to finite
        // numbers keeps its colour, see `char_style`).
        let to_text = xf.inverse();
        let chars: Vec<char> = text.chars().collect();
        let mut runs = vec![];
        let mut from = 0usize;
        for (n, st) in &self.runs {
            let to = from.saturating_add(*n).min(chars.len());
            let piece: String = chars.get(from..to).unwrap_or_default().iter().collect();
            from = to;
            if piece.is_empty() {
                continue;
            }
            runs.push(TextRun::new(piece, char_style(st, scale, to_text)));
        }
        if from < chars.len() {
            let last = self.runs.last().map(|r| &r.1)?;
            runs.push(TextRun::new(chars.get(from..).unwrap_or_default().iter().collect::<String>(), char_style(last, scale, to_text)));
        }
        let first = self.runs.first().map(|r| char_style(&r.1, scale, to_text))?;
        let mut t = TextObject::point(Point::ZERO, "", first);
        if !runs.is_empty() {
            t.runs = runs;
        }
        let paras: Vec<ParaStyle> = self.paragraph_styles(&text, scale);
        t.para = paras.first().cloned().unwrap_or_default();
        if paras.len() > 1 && paras.iter().any(|p| *p != t.para) {
            t.paras = paras;
        }
        if matches!(kind, TextKind::OnPath { .. }) && !threaded {
            t.path_spacing = self.path_spacing * scale;
        }
        // Each frame of a thread keeps its own options.
        match self.frames.get(frame) {
            Some(FrameKind::Area(_, options)) => options.apply(&mut t.area, scale),
            Some(FrameKind::OnPath { options, .. }) => {
                t.path_effect = options.effect;
                t.path_align = options.align;
            }
            _ => {}
        }
        t.vertical = self.vertical;
        t.kind = kind;
        t.xf = xf;
        Some(Node::new(id, NodeKind::Text(Box::new(t))))
    }

    /// Flips type on a path (`node`, made from frame `frame`) when its frame keeps Flip on. The
    /// frame's path runs as it did before the flip, so this runs once, on the path the object ends
    /// up with.
    pub(super) fn flip_on_path(&self, node: &mut Node, frame: usize) {
        if let Some(FrameKind::OnPath { options: PathFrame { flip: true, .. }, .. }) = self.frames.get(frame)
            && let NodeKind::Text(t) = &mut node.kind
        {
            t.flip_on_path();
        }
    }

    /// Area type that the file lays lines out in, in a frame too short for its first line as it is
    /// laid out here: a font that isn't available shows in the fallback font, whose lines may be
    /// taller, and type that doesn't fit its frame isn't drawn. The frame is made taller, from its
    /// top, by what the first line needs (`vectorcraft_text::fit_first_line`). Whether it was.
    pub(super) fn fit_first_line(&self, t: &mut TextObject) -> bool {
        !self.lines.is_empty() && vectorcraft_text::fit_first_line(vectorcraft_text::FontDb::global(), t)
    }

    /// Area type of one frame that the file lays more lines out in than fit here: in the fallback
    /// font, or a last line whose descent the file lets pass the frame's bottom. The frame is made
    /// taller from its bottom (`vectorcraft_text::fit_source_lines`) so the file's lines show.
    /// Whether it was.
    pub(super) fn fit_last_lines(&self, t: &mut TextObject) -> bool {
        matches!(self.frames.as_slice(), [FrameKind::Area(..)])
            && vectorcraft_text::fit_source_lines(vectorcraft_text::FontDb::global(), t, self.lines.len())
    }

    /// Were tab stops past [`MAX_STORY_TABS`] left out of its paragraphs (so far: making its text
    /// object counts them again)?
    pub(super) fn tabs_left_out(&self) -> bool {
        self.tabs_left_out.get()
    }

    /// Area type (`node`, the story's first frame as [`Self::node`] makes it with `template` and
    /// `to_doc`) whose first line the file lays out higher or lower than it is laid out here, as its
    /// page draws it: its first baseline is fixed where the file has it. The font's ascent below the
    /// frame's top, the first baseline here, isn't where the app that wrote the file puts it for
    /// every font. A first baseline its frame `frame` keeps (other than Ascent) stays as it is.
    /// Whether it was.
    pub(super) fn place_first_line(&self, node: &mut Node, frame: usize, template: (f64, f64), to_doc: Affine) -> bool {
        // Closer than this (points of the document) and the line is where the file has it.
        const SAME: f64 = 0.5;
        if matches!(self.frames.get(frame), Some(FrameKind::Area(_, AreaFrame { first_baseline: Some(_), .. }))) {
            return false;
        }
        let NodeKind::Text(t) = &mut node.kind else { return false };
        if !matches!(t.kind, TextKind::Area { .. }) || t.area.vertical_align != vectorcraft_doc::VerticalAlign::Top {
            return false;
        }
        let Some((_, (x, y))) = self.line_starts(template).into_iter().next() else { return false };
        let at = to_doc * Point::new(x, y);
        let db = vectorcraft_text::FontDb::global();
        // A frame too short for the line lays nothing out: as the line needs it (see `fit_first_line`).
        let mut probe = TextObject::clone(t);
        vectorcraft_text::fit_first_line(db, &mut probe);
        let laid = vectorcraft_text::layout(db, &probe);
        let Some(first) = laid.lines.first().filter(|l| l.region == 0) else { return false };
        // Where the first baseline goes in the layout's line space. Vertical type's line space is a
        // quarter turn from the document (a line's height is how far left of the frame's right edge
        // its column is), and the file's line is the column's centre line, the middle of the line
        // (as `TextLayout::baselines` has it): the baseline is that far from it.
        let want = if t.vertical { -at.x - (first.descent - first.ascent) / 2.0 } else { at.y };
        let min = first.first_baseline + (want - first.baseline);
        if (want - first.baseline).abs() <= SAME || !min.is_finite() || min <= 0.0 {
            return false;
        }
        t.area.first_baseline = vectorcraft_doc::FirstBaseline::Fixed;
        t.area.first_baseline_min = min;
        true
    }

    /// The style of each paragraph of `text` (paragraphs split at `\n`).
    fn paragraph_styles(&self, text: &str, scale: f64) -> Vec<ParaStyle> {
        let count = text.split('\n').count();
        let mut out = Vec::with_capacity(count);
        let mut runs = self.paras.iter();
        let mut current = runs.next();
        let mut left = current.map_or(usize::MAX, |r| r.0);
        let mut budget = MAX_STORY_TABS;
        for p in text.split('\n') {
            let mut style = current.map_or_else(|| para_style(&Para::default(), scale), |r| para_style(&r.1, scale));
            if spend_tabs(&mut style.tabs, &mut budget) {
                self.tabs_left_out.set(true);
            }
            out.push(style);
            // Each paragraph and its ending `\r` count.
            let mut used = p.chars().count() + 1;
            while used > 0 {
                if left > used {
                    left -= used;
                    used = 0;
                } else {
                    used -= left;
                    current = runs.next();
                    left = current.map_or(usize::MAX, |r| r.0);
                    if current.is_none() {
                        break;
                    }
                }
            }
        }
        out
    }
}

/// The paragraph style of paragraph attributes `p`, its lengths (indents, spacing, the
/// hyphenation zone) scaled by `scale`, as sizes are.
fn para_style(p: &Para, scale: f64) -> ParaStyle {
    ParaStyle {
        justify: match p.justify {
            0 if p.rtl => Justify::Right,
            1 if p.rtl => Justify::Left,
            1 => Justify::Right,
            2 => Justify::Center,
            3 => Justify::JustifyLeft,
            4 => Justify::JustifyRight,
            5 => Justify::JustifyCenter,
            6 => Justify::JustifyAll,
            _ => Justify::Left,
        },
        left_indent: p.left_indent * scale,
        right_indent: p.right_indent * scale,
        first_line_indent: p.first_indent * scale,
        space_before: p.space_before * scale,
        space_after: p.space_after * scale,
        hyphenate: p.hyphenate,
        hyphenation: HyphenationOptions { zone: p.hyphenation.zone * scale, ..p.hyphenation.clone() },
        tabs: p.tabs.iter().map(|t| TabStop { position: t.position * scale, ..t.clone() }).collect(),
        // A right-to-left paragraph lays its text out from the right whatever letters it has
        // (" 120.5 mm " reads "mm 120.5"); without the key, from its first strong letter.
        direction: p.rtl.then_some(ParaDirection::RightToLeft),
        justification: JustificationOptions {
            // Rounded, so the file's 1.2 is the default 120% exactly.
            auto_leading: p.auto_leading.map_or(JustificationOptions::default().auto_leading, |a| (a * 1e6).round() / 1e4),
            ..JustificationOptions::default()
        },
        ..ParaStyle::default()
    }
}

/// The character style of character attributes `s`: its font from the fonts the app has, its
/// lengths (size, leading, baseline shift, stroke width) scaled by `scale`, and its patterns placed
/// in the type's text space by `to_text` (the document → its text space), where they stay as the
/// document has them.
fn char_style(s: &Style, scale: f64, to_text: Affine) -> CharStyle {
    let (family, style) = s.font.as_deref().map_or_else(|| (CharStyle::default().font_family, "Regular".to_string()), crate::family_style);
    let family = vectorcraft_text::FontDb::global().find_family(&family).unwrap_or(family);
    // A fill or stroke that is on takes its pattern in place of its colour.
    let paint = |color: Option<Color>, pattern: &Option<Paint>| match (color, pattern) {
        // A pattern that can't be placed (the type's text space out of range) keeps the colour.
        (Some(_), Some(Paint::Pattern { pattern, xf })) if (to_text * *xf).is_finite() => {
            Paint::Pattern { pattern: pattern.clone(), xf: to_text * *xf }
        }
        (color, _) => color.map_or(Paint::None, Paint::solid),
    };
    // Type without a stroke keeps the stroke's defaults, as its weight is 0.
    let (stroke_cap, stroke_join, stroke_miter_limit) =
        if s.stroke.is_some() { (s.stroke_cap, s.stroke_join, s.stroke_miter_limit) } else { (LineCap::Butt, LineJoin::Miter, 10.0) };
    CharStyle {
        font_family: family,
        font_style: style,
        size: s.size * scale,
        // Auto leading stays auto: the layout gives each line the paragraph's share of the largest
        // type on it, as the file's own layout does ("The largest leading value in a line of type
        // determines the leading for that line": https://helpx.adobe.com/illustrator/using/line-character-spacing.html).
        leading: s.leading.map(|l| l * scale),
        tracking: s.tracking,
        baseline_shift: s.baseline_shift * scale,
        h_scale: s.h_scale * 100.0,
        v_scale: s.v_scale * 100.0,
        fill: paint(s.fill, &s.fill_pattern),
        stroke: paint(s.stroke, &s.stroke_pattern),
        stroke_width: if s.stroke.is_some() { s.stroke_width * scale } else { 0.0 },
        stroke_cap,
        stroke_join,
        stroke_miter_limit,
        all_caps: s.caps == 2,
        // At the default Small Caps size of Document Setup: the file's own isn't read.
        small_caps: (s.caps == 1).then(|| vectorcraft_doc::DocSetup::default().small_caps_size),
        tate_chu_yoko: s.tate_chu_yoko,
        standard_vertical_roman: s.standard_vertical_roman,
        ..CharStyle::default()
    }
}

/// How many document points one unit of the art is: `to_doc`'s scale, which carries the art's
/// lengths onto the document (10 for a Large Canvas copy written at 1:10; see `ai::read`).
fn art_scale(to_doc: Affine) -> f64 {
    let [a, b, ..] = to_doc.as_coeffs();
    (a * a + b * b).sqrt().max(1e-9)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn right_to_left_paragraphs_swap_left_and_right_alignment() {
        let texts = Texts {
            stories: vec![],
            fonts: vec![],
            defaults: Val::Dict(BTreeMap::new()),
            para_defaults: read(b"/33 1").unwrap(),
            char_sheets: vec![],
            para_sheets: vec![],
            frames: vec![],
            patterns: HashMap::new(),
        };
        let justify = |own: &[u8]| para_style(&texts.para(Some(&read(&[b"/5 << ", own, b" >>"].concat()).unwrap())), 1.0).justify;
        // Right to left from the document's defaults, or the paragraph's own.
        assert_eq!(justify(b"/0 0"), Justify::Right);
        assert_eq!(justify(b"/0 1 /33 1"), Justify::Left);
        assert_eq!(justify(b"/0 2"), Justify::Center);
        // Left to right keeps them.
        assert_eq!(justify(b"/0 0 /33 0"), Justify::Left);
        assert_eq!(justify(b"/0 1 /33 0"), Justify::Right);
    }

    /// Key 7 is the paragraph's auto leading, a share of the type size; one that can't be read
    /// leaves the default, 120%.
    #[test]
    fn a_paragraph_keeps_its_auto_leading() {
        let texts = Texts {
            stories: vec![],
            fonts: vec![],
            defaults: Val::Dict(BTreeMap::new()),
            para_defaults: read(b"/7 1.2").unwrap(),
            char_sheets: vec![],
            para_sheets: vec![],
            frames: vec![],
            patterns: HashMap::new(),
        };
        let auto = |own: &[u8]| para_style(&texts.para(Some(&read(&[b"/5 << ", own, b" >>"].concat()).unwrap())), 1.0).justification.auto_leading;
        assert_eq!(auto(b""), JustificationOptions::default().auto_leading);
        assert_eq!(auto(b"/7 1.5"), 150.0);
        assert_eq!(auto(b"/7 1.15"), 115.0);
        for unreadable in [b"/7 0".as_slice(), b"/7 -1", b"/7 1e9", b"/7 /nil"] {
            assert!((auto(unreadable) - 120.0).abs() < 1e-9, "{}", String::from_utf8_lossy(unreadable));
        }
    }

    /// Keys 10 to 15 are the paragraph's hyphenation options; one that can't be read, or is out of
    /// its range, keeps the default. The zone is in points of the text document, scaled as sizes are.
    #[test]
    fn a_paragraph_keeps_its_hyphenation_options() {
        let texts = Texts {
            stories: vec![],
            fonts: vec![],
            defaults: Val::Dict(BTreeMap::new()),
            para_defaults: read(b"/10 6 /11 2 /12 2 /13 0 /14 36.0 /15 true").unwrap(),
            char_sheets: vec![],
            para_sheets: vec![],
            frames: vec![],
            patterns: HashMap::new(),
        };
        let options = |own: &[u8], scale: f64| para_style(&texts.para(Some(&read(&[b"/5 << ", own, b" >>"].concat()).unwrap())), scale).hyphenation;
        let want = HyphenationOptions { words_longer_than: 6, after_first: 2, before_last: 2, hyphen_limit: 0, zone: 36.0, capitalized: true };
        assert_eq!(options(b"", 1.0), want);
        assert_eq!(
            options(b"/10 8 /13 3 /15 false", 0.5),
            HyphenationOptions { words_longer_than: 8, hyphen_limit: 3, zone: 18.0, capitalized: false, ..want.clone() }
        );
        let d = HyphenationOptions::default();
        let bare = Texts {
            stories: vec![],
            fonts: vec![],
            defaults: Val::Dict(BTreeMap::new()),
            para_defaults: Val::Dict(BTreeMap::new()),
            char_sheets: vec![],
            para_sheets: vec![],
            frames: vec![],
            patterns: HashMap::new(),
        };
        let own = |own: &[u8]| para_style(&bare.para(Some(&read(&[b"/5 << ", own, b" >>"].concat()).unwrap())), 1.0).hyphenation;
        assert_eq!(own(b""), d);
        assert_eq!(own(b"/10 0 /11 99 /12 2.5 /13 -1 /14 -3 /15 1"), d);
    }

    /// Key 30 holds the paragraph's tab stops: their positions (key 0), sorted, and alignments (key
    /// 1: 0 left, 1 center, 2 right, 3 decimal), scaled as sizes are. A stop that can't be read is
    /// left out. A user's price list keeps a right stop at 269.29 and a left one at 272.13.
    #[test]
    fn a_paragraph_keeps_its_tab_stops() {
        let texts = Texts {
            stories: vec![],
            fonts: vec![],
            defaults: Val::Dict(BTreeMap::new()),
            para_defaults: Val::Dict(BTreeMap::new()),
            char_sheets: vec![],
            para_sheets: vec![],
            frames: vec![],
            patterns: HashMap::new(),
        };
        let tabs = |own: &[u8], scale: f64| {
            para_style(&texts.para(Some(&read(&[b"/5 << ", own, b" >>"].concat()).unwrap())), scale)
                .tabs
                .iter()
                .map(|t| (t.position, t.align))
                .collect::<Vec<_>>()
        };
        assert_eq!(tabs(b"", 1.0), []);
        assert_eq!(
            tabs(b"/30 << /0 [ << /0 272.12598 /1 0 >> << /0 269.29135 /1 2 >> ] >>", 1.0),
            [(269.29135, TabAlign::Right), (272.12598, TabAlign::Left)]
        );
        assert_eq!(
            tabs(b"/30 << /0 [ << /0 10 /1 1 >> << /0 20 /1 3 >> << /0 30 >> ] >>", 0.5),
            [(5.0, TabAlign::Center), (10.0, TabAlign::Decimal), (15.0, TabAlign::Left)]
        );
        // Past MAX_TABS stops, the rest are left out, and the paragraph says so (the story's warning).
        let many = |n: usize| {
            let stops: String = (0..n).map(|i| format!("<< /0 {i} >> ")).collect();
            texts.para(Some(&read(format!("/5 << /30 << /0 [ {stops}] >> >>").as_bytes()).unwrap()))
        };
        let p = many(MAX_TABS + 6);
        assert!(p.tabs_truncated && p.tabs.len() == MAX_TABS);
        let p = many(MAX_TABS);
        assert!(!p.tabs_truncated && p.tabs.len() == MAX_TABS);
        // Unreadable: a position out of range or not a number, an unknown alignment.
        assert_eq!(
            tabs(b"/30 << /0 [ << /0 -1 >> << /0 1e9 >> << /0 /nil >> << /0 5 /1 7 >> << /0 6 /1 /x >> << /0 7 /1 2 >> ] >>", 1.0),
            [(7.0, TabAlign::Right)]
        );
    }

    /// A story's paragraphs take at most [`MAX_STORY_TABS`] tab stops in all (a paragraph style's
    /// are copied into each paragraph): those past it take the default stops, and the story says so.
    #[test]
    fn a_story_s_tab_stops_are_bounded() {
        let mut budget = 3;
        let mut tabs = vec![TabStop { position: 1.0, align: TabAlign::Left, leader: String::new(), align_on: '.' }; 2];
        assert!(!spend_tabs(&mut tabs, &mut budget) && tabs.len() == 2 && budget == 1);
        assert!(spend_tabs(&mut tabs, &mut budget) && tabs.is_empty() && budget == 1);
        // Paragraphs of a style with MAX_TABS stops each, more than the story takes.
        let stop = TabStop { position: 1.0, align: TabAlign::Left, leader: String::new(), align_on: '.' };
        let para = Para { tabs: vec![stop; MAX_TABS], ..Para::default() };
        let count = MAX_STORY_TABS / MAX_TABS + 2;
        let s = story(&"a\r".repeat(count), &[0.0], &para);
        assert!(!s.tabs_left_out());
        let styles = s.paragraph_styles(&format!("{}a", "a\n".repeat(count - 1)), 1.0);
        assert_eq!(styles.len(), count);
        assert!(s.tabs_left_out());
        assert_eq!(styles.iter().map(|p| p.tabs.len()).sum::<usize>(), MAX_STORY_TABS);
        assert!(styles.last().is_some_and(|p| p.tabs.is_empty()));
    }

    /// Where point type lies runs along its baseline: a label turned a quarter (as a vertical
    /// dimension's is) lies in a tall box, not a wide one.
    #[test]
    fn point_type_lies_along_its_baseline() {
        let para = Texts {
            stories: vec![],
            fonts: vec![],
            defaults: Val::Dict(BTreeMap::new()),
            para_defaults: Val::Dict(BTreeMap::new()),
            char_sheets: vec![],
            para_sheets: vec![],
            frames: vec![],
            patterns: HashMap::new(),
        }
        .para(None);
        let mut s = story(" 80 mm ", &[0.0], &para);
        let size = |s: &Story| s.region((0.0, 0.0), Affine::IDENTITY).map(|r| (r.width(), r.height())).unwrap();
        let (w, h) = size(&s);
        assert!(w > 3.0 * h, "level: {w} × {h}");
        // Turned −90° on the canvas (y down): the baseline runs down the page.
        s.frames = vec![FrameKind::Point([0.0, 1.0, -1.0, 0.0, 0.0, 0.0])];
        let (w, h) = size(&s);
        assert!(h > 3.0 * w, "turned: {w} × {h}");
        // Vertical type lies down the page without a turn.
        s.frames = vec![FrameKind::Point([1.0, 0.0, 0.0, 1.0, 0.0, 0.0])];
        s.vertical = true;
        let (w, h) = size(&s);
        assert!(h > 3.0 * w, "vertical: {w} × {h}");
    }

    /// A vertical layout's lines run down the canvas from the anchor, each next one further left:
    /// the second column starts one line pitch left of the first, and a centred column (its start
    /// before the anchor) starts higher up.
    #[test]
    fn vertical_lines_start_down_the_canvas_each_further_left() {
        let mut s = story("ab\rc\rde\r", &[0.0, 14.4, 28.8], &Para::default());
        s.vertical = true;
        s.lines[2].0 = -12.0;
        s.anchor = (CANVAS_CENTRE + 100.0, CANVAS_CENTRE - 50.0);
        let starts: Vec<(f64, f64)> = s.line_starts((0.0, 0.0)).into_iter().map(|(_, p)| p).collect();
        let [a, b, c] = starts.as_slice() else { panic!("{starts:?}") };
        assert!((a.0 - 100.0).abs() < 1e-9 && (a.1 - 50.0).abs() < 1e-9, "the first column at the anchor: {a:?}");
        assert!((b.0 - 85.6).abs() < 1e-9 && (b.1 - 50.0).abs() < 1e-9, "the next one to the left: {b:?}");
        assert!((c.0 - 71.2).abs() < 1e-9 && (c.1 - 62.0).abs() < 1e-9, "a centred one higher (the art is y up): {c:?}");
    }

    /// Key 1 of what a frame carries is its orientation: 2 vertical; 0, none or any other value
    /// horizontal.
    #[test]
    fn a_frame_is_vertical_only_at_orientation_two() {
        let frame = |carries: &str| read(format!("/0 << /2 << /0 0 {carries} >> >>").as_bytes()).unwrap();
        let cases = ["/1 2", "/1 0", "", "/1 1", "/1 2.5", "/1 -2", "/1 (2)", "/1 1e300", "/1 true", "/1 [ 2 ]"];
        let texts = Texts {
            stories: vec![],
            fonts: vec![],
            defaults: Val::Dict(BTreeMap::new()),
            para_defaults: Val::Dict(BTreeMap::new()),
            char_sheets: vec![],
            para_sheets: vec![],
            frames: cases.iter().map(|c| frame(c)).collect(),
            patterns: HashMap::new(),
        };
        let vertical: Vec<bool> = (0..cases.len() + 1).map(|i| texts.frame_vertical(i)).collect();
        assert_eq!(vertical, [true, false, false, false, false, false, false, false, false, false, false]);
    }

    /// Key 35 of a character style at 3 is Tate-chu-yoko; its other values (1 and 2, how half-width
    /// characters stand in a column) and values that aren't that number aren't.
    #[test]
    fn baseline_direction_three_is_tate_chu_yoko() {
        let texts = Texts {
            stories: vec![],
            fonts: vec![],
            defaults: read(b"/35 1").unwrap(),
            para_defaults: Val::Dict(BTreeMap::new()),
            char_sheets: vec![read(b"/0 ([Normal Character Style]) /6 << >>").unwrap(), read(b"/0 (Block) /6 << /35 3 >>").unwrap()],
            para_sheets: vec![],
            frames: vec![],
            patterns: HashMap::new(),
        };
        let tcy = |run: &[u8]| texts.style(Some(&read(run).unwrap())).tate_chu_yoko;
        assert!(tcy(b"/6 << /35 3 >>"), "the run's own");
        assert!(tcy(b"/5 1 /6 << >>"), "its character style's");
        assert!(!tcy(b"/5 1 /6 << /35 1 >>"), "the run's own wins");
        for other in [&b"/6 << >>"[..], b"/6 << /35 2 >>", b"/6 << /35 3.5 >>", b"/6 << /35 (3) >>", b"/6 << /35 -3 >>", b"/6 << /35 1e308 >>"] {
            assert!(!tcy(other), "{}", String::from_utf8_lossy(other));
        }
    }

    /// Key 35 of a character style at 1 (Standard) is Standard Vertical Roman Alignment, from the
    /// run, its character style or the document's defaults; at 2 (rotated), 3 (Tate-chu-yoko), a
    /// value that isn't one of those numbers, or with no key anywhere it is off.
    #[test]
    fn baseline_direction_one_is_standard_vertical_roman_alignment() {
        let texts = |defaults: &[u8]| Texts {
            stories: vec![],
            fonts: vec![],
            defaults: read(defaults).unwrap(),
            para_defaults: Val::Dict(BTreeMap::new()),
            char_sheets: vec![read(b"/0 ([Normal Character Style]) /6 << >>").unwrap(), read(b"/0 (Rotated) /6 << /35 2 >>").unwrap()],
            para_sheets: vec![],
            frames: vec![],
            patterns: HashMap::new(),
        };
        let roman = |defaults: &[u8], run: &[u8]| texts(defaults).style(Some(&read(run).unwrap())).standard_vertical_roman;
        assert!(roman(b"/35 1", b"/6 << >>"), "the document's defaults");
        assert!(roman(b"/35 2", b"/6 << /35 1 >>"), "the run's own");
        assert!(!roman(b"/35 1", b"/6 << /35 2 >>"), "the run's own rotated");
        assert!(!roman(b"/35 1", b"/5 1 /6 << >>"), "its character style's");
        assert!(roman(b"/35 2", b"/5 1 /6 << /35 1 >>"), "the run's own wins over its style");
        assert!(!roman(b"/0 0", b"/6 << >>"), "no key: off");
        for other in
            [&b"/6 << /35 3 >>"[..], b"/6 << /35 1.5 >>", b"/6 << /35 (1) >>", b"/6 << /35 -1 >>", b"/6 << /35 true >>", b"/6 << /35 [ 1 ] >>"]
        {
            assert!(!roman(b"/0 0", other), "{}", String::from_utf8_lossy(other));
        }
        assert!(texts(b"/35 3").style(None).tate_chu_yoko && !texts(b"/35 3").style(None).standard_vertical_roman);
    }

    /// Keys 61, 62 and 64 of a character style are its stroke's cap (0 butt, 1 round, 2
    /// projecting), join (0 miter, 1 round, 2 bevel) and miter limit, from the run, its character
    /// style or the document's defaults. A value that isn't one of theirs keeps the default, a miter
    /// limit past 500 is 500, and type without a stroke keeps the defaults.
    #[test]
    fn a_run_keeps_its_stroke_cap_join_and_miter_limit() {
        let texts = Texts {
            stories: vec![],
            fonts: vec![],
            defaults: read(b"/57 true /54 << /0 << /0 0 /1 [ 1.0 1.0 ] >> >> /64 4.0").unwrap(),
            para_defaults: Val::Dict(BTreeMap::new()),
            char_sheets: vec![read(b"/0 ([Normal Character Style]) /6 << >>").unwrap(), read(b"/0 (Rounded) /6 << /61 1 /62 1 >>").unwrap()],
            para_sheets: vec![],
            frames: vec![],
            patterns: HashMap::new(),
        };
        let stroke = |run: &[u8]| {
            let s = char_style(&texts.style(Some(&read(run).unwrap())), 1.0, Affine::IDENTITY);
            (s.stroke_cap, s.stroke_join, s.stroke_miter_limit)
        };
        assert_eq!(stroke(b"/6 << /61 2 /62 2 /64 2.5 >>"), (LineCap::Square, LineJoin::Bevel, 2.5), "the run's own");
        assert_eq!(stroke(b"/5 1 /6 << >>"), (LineCap::Round, LineJoin::Round, 4.0), "its character style's and the defaults'");
        assert_eq!(stroke(b"/5 1 /6 << /62 0 >>"), (LineCap::Round, LineJoin::Miter, 4.0), "the run's own wins over its style");
        assert_eq!(stroke(b"/6 << >>"), (LineCap::Butt, LineJoin::Miter, 4.0), "the defaults'");
        assert_eq!(stroke(b"/6 << /64 1 >>").2, 1.0);
        assert_eq!(stroke(b"/6 << /64 1e9 >>").2, 500.0);
        for bad in [
            &b"/6 << /61 3 /62 3 /64 0.5 >>"[..],
            b"/6 << /61 -1 /62 -1 /64 -4 >>",
            b"/6 << /61 1.5 /62 0.5 /64 NaN >>",
            b"/6 << /61 (1) /62 (1) /64 inf >>",
            b"/6 << /61 true /62 [ 1 ] /64 (4) >>",
            b"/6 << /61 1e308 /62 /Round /64 -inf >>",
        ] {
            assert_eq!(stroke(bad), (LineCap::Butt, LineJoin::Miter, 10.0), "{}", String::from_utf8_lossy(bad));
        }
        assert_eq!(stroke(b"/5 1 /6 << /57 false >>"), (LineCap::Butt, LineJoin::Miter, 10.0), "no stroke");
    }

    /// The pattern "Dots", its tile's corner at `at`.
    fn dots_at(at: (f64, f64)) -> Paint {
        Paint::Pattern { pattern: "Dots".into(), xf: Affine::translate(at) }
    }

    /// Texts whose characters may name the art style "Anon 1", which paints with "Dots" at (10, 20)
    /// in the document, and "Solid", which isn't a pattern.
    fn patterned_texts() -> Texts {
        let patterns = HashMap::from([("Anon 1".to_string(), dots_at((10.0, 20.0))), ("Solid".to_string(), Paint::solid(Color::WHITE))]);
        Texts {
            stories: vec![],
            fonts: vec![],
            defaults: Val::Dict(BTreeMap::new()),
            para_defaults: Val::Dict(BTreeMap::new()),
            char_sheets: vec![],
            para_sheets: vec![],
            frames: vec![],
            patterns: HashMap::new(),
        }
        .with_patterns(patterns)
    }

    /// Key 5 of a run's fill or stroke names the art style that paints it: one that paints with a
    /// pattern gives the run that pattern in place of the colour of key 0 (black), placed in the
    /// type's text space where the document has it. A fill or stroke that is off stays off, and a
    /// key 5 that isn't the name of such a style, or isn't a name at all, keeps the colour.
    #[test]
    fn a_run_painted_by_a_patterns_art_style_takes_the_pattern() {
        // GUARDS: type a pattern paints came in black, the colour key 0 keeps for it.
        let texts = patterned_texts();
        let black_paint = Paint::solid(Color::gray(1.0));
        let to_text = Affine::translate((-4.0, -5.0));
        let paints = |run: &str| {
            let s = char_style(&texts.style(Some(&read(run.as_bytes()).unwrap())), 1.0, to_text);
            (s.fill, s.stroke)
        };
        let black = |style: &str| format!("<< /99 /CAITextPaint /0 << /0 0 /1 [ 1.0 0.0 ] >> {style} >>");
        let fill = |style: &str| paints(&format!("/6 << /53 {} >>", black(style)));
        assert_eq!(fill("/5 (Anon 1)"), (dots_at((6.0, 15.0)), Paint::None));
        let stroke = paints(&format!("/6 << /53 {} /57 true /54 {} >>", black(""), black("/5 (Anon 1)")));
        assert_eq!(stroke, (black_paint.clone(), dots_at((6.0, 15.0))));
        // Off.
        assert_eq!(paints(&format!("/6 << /56 false /53 {} >>", black("/5 (Anon 1)"))), (Paint::None, Paint::None));
        assert_eq!(paints(&format!("/6 << /57 false /54 {} >>", black("/5 (Anon 1)"))).1, Paint::None);
        // A stroke whose colour can't be read paints nothing, pattern or not, as before.
        assert_eq!(paints("/6 << /57 true /54 << /0 << /0 9 /1 [ 1.0 ] >> /5 (Anon 1) >> >>").1, Paint::None);
        for other in ["", "/5 (Anon 2)", "/5 (Solid)", "/5 ()", "/5 /Anon", "/5 1", "/5 [ (Anon 1) ]", "/5 << /0 (Anon 1) >>", "/5 true"] {
            assert_eq!(fill(other), (black_paint.clone(), Paint::None), "{other}");
        }
        // A text space the pattern can't be placed in keeps the colour.
        let run = read(format!("/6 << /53 {} >>", black("/5 (Anon 1)")).as_bytes()).unwrap();
        for to_text in [Affine::new([1e308, 0.0, 0.0, 1e308, 0.0, 0.0]), Affine::new([f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0])] {
            assert_eq!(char_style(&texts.style(Some(&run)), 1.0, to_text).fill, black_paint.clone(), "{to_text:?}");
        }
    }

    /// A run's pattern stays where the document has it whatever the type's own text space: turned
    /// and moved point type, and area type, whose text space is the document's.
    #[test]
    fn a_runs_pattern_stays_where_the_document_has_it() {
        let to_doc = Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 100.0]);
        for frame in [
            FrameKind::Point([0.0, 1.0, -1.0, 0.0, 30.0, 40.0]),
            FrameKind::Point([2.0, 0.0, 0.0, 2.0, -7.0, 3.0]),
            FrameKind::Area(area_segments(), AreaFrame::default()),
        ] {
            let mut st = story("type", &[0.0], &Para::default());
            st.frames = vec![frame.clone()];
            st.runs[0].1.fill_pattern = Some(dots_at((10.0, 20.0)));
            st.runs[0].1.stroke = Some(Color::BLACK);
            st.runs[0].1.stroke_pattern = Some(dots_at((-3.0, 8.0)));
            let node = st.node(NodeId(1), 0, (0.0, 0.0), to_doc).unwrap();
            let NodeKind::Text(t) = node.kind else { panic!("not type") };
            for (paint, at) in [(&t.runs[0].style.fill, Point::new(10.0, 20.0)), (&t.runs[0].style.stroke, Point::new(-3.0, 8.0))] {
                let Paint::Pattern { pattern, xf } = paint else { panic!("{paint:?}") };
                assert_eq!(pattern, "Dots");
                let corner = t.xf * *xf * Point::ZERO;
                assert!((corner - at).hypot() < 1e-9, "{frame:?}: {corner:?}");
            }
        }
    }

    /// A right-to-left paragraph is laid out right to left even when it has no right-to-left
    /// letters (a dimension's label " 120.5 mm " reads "mm 120.5"); a left-to-right one, or one
    /// that doesn't say, goes by its letters.
    #[test]
    fn right_to_left_paragraphs_set_their_direction() {
        let texts = Texts {
            stories: vec![],
            fonts: vec![],
            defaults: Val::Dict(BTreeMap::new()),
            para_defaults: read(b"/33 1").unwrap(),
            char_sheets: vec![],
            para_sheets: vec![],
            frames: vec![],
            patterns: HashMap::new(),
        };
        let direction = |own: &[u8]| para_style(&texts.para(Some(&read(&[b"/5 << ", own, b" >>"].concat()).unwrap())), 1.0).direction;
        assert_eq!(direction(b"/0 2"), Some(ParaDirection::RightToLeft), "from the document's defaults");
        assert_eq!(direction(b"/0 2 /33 0"), None, "the paragraph's own");
        let plain = Texts { para_defaults: Val::Dict(BTreeMap::new()), ..texts };
        assert_eq!(para_style(&plain.para(None), 1.0).direction, None);
    }

    #[test]
    fn values_read_nested_dictionaries_lists_and_utf16_strings() {
        let v = read(b" /1 << /0 [ 1 2.5 -3 ] /2 (\xfe\xff\x00h\x00i) /3 true /99 /Name >> /0 (a\\)b\\n)").unwrap();
        let one = v.get("1").unwrap();
        assert_eq!(one.get("0").unwrap().nums(), Some(vec![1.0, 2.5, -3.0]));
        assert_eq!(one.get("2").unwrap().str(), Some("hi"));
        assert_eq!(one.get("3"), Some(&Val::Bool(true)));
        assert!(one.is_node("Name"));
        assert_eq!(v.get("0").unwrap().str(), Some("a)b\n"));
    }

    #[test]
    fn damaged_documents_are_not_read() {
        for bad in [&b"/0 << /1"[..], b"/0 [ 1 2", b"/0 (abc", b"/0 nope", b"0 1"] {
            assert!(read(bad).is_none(), "{}", String::from_utf8_lossy(bad));
        }
        let deep = format!("/0 {}", "[".repeat(200));
        assert!(read(deep.as_bytes()).is_none());
    }

    /// A text document's frame on a path of `segments` (each `x0 y0 x1 y1 x2 y2 x3 y3`) with the
    /// range `range`, written by hand.
    fn on_path_frame(segments: &[[f64; 8]], range: [f64; 2]) -> Option<FrameKind> {
        on_path_frame_with(segments, range, "")
    }

    /// The same with `options` in the frame data too.
    fn on_path_frame_with(segments: &[[f64; 8]], range: [f64; 2], options: &str) -> Option<FrameKind> {
        let nums = |v: &[f64]| v.iter().map(|n| format!("{n}")).collect::<Vec<_>>().join(" ");
        let all: Vec<f64> = segments.iter().flatten().copied().collect();
        let doc = format!("/0 << /1 << /0 [ {} ] >> /2 << /0 2 /6 [ {} ] {options} >> >>", nums(&all), nums(&range));
        let texts = Texts {
            stories: vec![],
            fonts: vec![],
            defaults: Val::Dict(BTreeMap::new()),
            para_defaults: Val::Dict(BTreeMap::new()),
            char_sheets: vec![],
            para_sheets: vec![],
            frames: vec![read(doc.as_bytes()).unwrap()],
            patterns: HashMap::new(),
        };
        texts.frame(0)
    }

    /// A circle of radius 100 round the origin in four cubics, from its right, closed.
    fn circle() -> Vec<[f64; 8]> {
        let k = 55.228475;
        vec![
            [100.0, 0.0, 100.0, k, k, 100.0, 0.0, 100.0],
            [0.0, 100.0, -k, 100.0, -100.0, k, -100.0, 0.0],
            [-100.0, 0.0, -100.0, -k, -k, -100.0, 0.0, -100.0],
            [0.0, -100.0, k, -100.0, 100.0, -k, 100.0, 0.0],
        ]
    }

    /// The text object of type `text` on `frame`, and how much of its path its brackets span.
    fn on_path(text: &str, frame: FrameKind) -> (TextObject, f64) {
        let mut st = story(text, &[0.0], &Para::default());
        st.frames = vec![frame];
        let node = st.node(NodeId(1), 0, (0.0, 0.0), Affine::IDENTITY).unwrap();
        let NodeKind::Text(t) = node.kind else { panic!("not type") };
        let (a, b) = t.kind.path_span().unwrap();
        (*t, b - a)
    }

    /// Key 11's codes for the effect and the alignment, each read on its own. The defaults are the
    /// keys left out.
    #[test]
    fn type_on_a_path_keeps_its_effect_and_alignment() {
        use PathAlign as A;
        use PathEffect as E;
        let read = |options: &str| {
            let (t, _) = on_path("type", on_path_frame_with(&circle(), [0.0, 3.99], options).unwrap());
            (t.path_effect, t.path_align)
        };
        assert_eq!(read(""), (E::Rainbow, A::Baseline));
        assert_eq!(read("/11 << /4 21 /18 21.0 /6 false /22 0.025 >>"), (E::Rainbow, A::Baseline));
        for (code, effect) in [(1, E::Skew), (2, E::Ribbon3d), (3, E::StairStep), (4, E::Gravity)] {
            assert_eq!(read(&format!("/11 << /1 {code} >>")), (effect, A::Baseline), "effect {code}");
        }
        for (code, align) in [(0, A::Ascender), (1, A::Descender), (2, A::Center)] {
            assert_eq!(read(&format!("/11 << /2 {code} >>")), (E::Rainbow, align), "alignment {code}");
        }
        assert_eq!(read("/11 << /1 3 /2 1 >>"), (E::StairStep, A::Descender));
    }

    /// A frame with key 6 true keeps the defaults, Flip included.
    #[test]
    fn type_on_a_path_with_key_6_true_keeps_the_defaults() {
        let Some(FrameKind::OnPath { options, .. }) = on_path_frame_with(&circle(), [0.0, 3.99], "/11 << /6 true /0 true /1 1 /2 0 >>") else {
            panic!("on a path")
        };
        assert_eq!(options, PathFrame::default());
    }

    /// Key 11 where it can't be read leaves the options at their defaults, each on its own.
    #[test]
    fn type_on_a_path_options_that_cannot_be_read_keep_their_defaults() {
        let bad = ["5", "-1", "1.5", "1e300", "(1)", "true", "/Skew", "[ 1 ]", "<< /0 1 >>"];
        for value in bad {
            for key in ["1", "2", "0"] {
                let Some(FrameKind::OnPath { options, .. }) = on_path_frame_with(&circle(), [0.0, 3.99], &format!("/11 << /{key} {value} >>")) else {
                    panic!("on a path")
                };
                let expected = PathFrame { flip: key == "0" && value == "true", ..PathFrame::default() };
                assert_eq!(options, expected, "key {key} value {value}");
            }
        }
        for whole in ["3", "true", "(x)", "[ /1 1 ]"] {
            let Some(FrameKind::OnPath { options, .. }) = on_path_frame_with(&circle(), [0.0, 3.99], &format!("/11 {whole}")) else {
                panic!("on a path")
            };
            assert_eq!(options, PathFrame::default(), "{whole}");
        }
        // One option that can't be read leaves the other as the frame keeps it.
        let Some(FrameKind::OnPath { options, .. }) = on_path_frame_with(&circle(), [0.0, 3.99], "/11 << /1 9 /2 2 >>") else { panic!("on a path") };
        assert_eq!((options.effect, options.align), (PathEffect::Rainbow, PathAlign::Center));
    }

    /// The first point of type on a path's path, and its span.
    fn flipped_on_path(frames: Vec<FrameKind>, frame: usize) -> (f64, (f64, f64)) {
        let mut st = story("type", &[0.0], &Para::default());
        st.frames = frames;
        let mut node = st.node(NodeId(1), frame, (0.0, 0.0), Affine::IDENTITY).unwrap();
        st.flip_on_path(&mut node, frame);
        let NodeKind::Text(t) = node.kind else { panic!("not type") };
        let vectorcraft_doc::TextKind::OnPath { path, .. } = &t.kind else { panic!("type on a path") };
        let Some(kurbo::PathEl::MoveTo(first)) = path.to_bezpath().elements().first().copied() else { panic!("a path") };
        (first.x + CANVAS_CENTRE, t.kind.path_span().unwrap())
    }

    /// Flip is a flag: the frame's path and range run as before the flip, so the type flips once,
    /// keeps its stretch of the path, and the path runs from its far end.
    #[test]
    fn flipped_type_on_a_path_runs_the_path_the_other_way_once() {
        let open = [[0.0, 0.0, 30.0, 0.0, 70.0, 0.0, 100.0, 0.0], [100.0, 0.0, 130.0, 0.0, 170.0, 0.0, 200.0, 0.0]];
        let frame = |options: &str| vec![on_path_frame_with(&open, [0.5, 1.5], options).unwrap()];
        let (first, (from, to)) = flipped_on_path(frame("/11 << /0 false >>"), 0);
        assert_eq!(first, 0.0);
        assert!((from - 0.25).abs() < 1e-3 && (to - 0.75).abs() < 1e-3, "{from} {to}");
        let (first, (from, to)) = flipped_on_path(frame("/11 << /0 true >>"), 0);
        assert_eq!(first, 200.0, "from the path's far end");
        assert!((from - 0.25).abs() < 1e-3 && (to - 0.75).abs() < 1e-3, "the same stretch: {from} {to}");
    }

    /// The end of an open path's range, which doesn't move unflipped type, is where flipped type
    /// starts: a range that ends just short of the path's end starts the type that far from it.
    #[test]
    fn flipped_type_on_an_open_path_starts_at_the_end_of_its_range() {
        let third = 100.0 / 3.0;
        let line = [[0.0, 0.0, third, 0.0, 2.0 * third, 0.0, 100.0, 0.0]];
        let frame = |options: &str| vec![on_path_frame_with(&line, [0.0, 0.99], options).unwrap()];
        let (_, (from, to)) = flipped_on_path(frame("/11 << /0 true >>"), 0);
        assert!((from - 0.01).abs() < 1e-3 && (to - 1.0).abs() < 1e-3, "{from} {to}");
        let (_, (from, to)) = flipped_on_path(frame(""), 0);
        assert!(from.abs() < 1e-3 && (to - 1.0).abs() < 1e-3, "unflipped: {from} {to}");
    }

    /// Round a closed path, a flip keeps the stretch too, and each frame of a thread keeps its own
    /// options.
    #[test]
    fn each_frame_of_type_on_a_path_keeps_its_options() {
        let mut st = story("type", &[0.0], &Para::default());
        st.frames = vec![
            on_path_frame_with(&circle(), [0.5, 2.5], "/11 << /0 true /1 1 >>").unwrap(),
            on_path_frame_with(&circle(), [0.0, 3.99], "/11 << /2 0 >>").unwrap(),
        ];
        let mut first = st.node(NodeId(1), 0, (0.0, 0.0), Affine::IDENTITY).unwrap();
        let mut second = st.node(NodeId(2), 1, (0.0, 0.0), Affine::IDENTITY).unwrap();
        let before = first.clone();
        st.flip_on_path(&mut first, 0);
        assert_ne!(before, first, "the first frame flips");
        let before = second.clone();
        st.flip_on_path(&mut second, 1);
        assert_eq!(before, second, "the second frame doesn't");
        let (NodeKind::Text(a), NodeKind::Text(b)) = (&first.kind, &second.kind) else { panic!("not type") };
        assert_eq!((a.path_effect, a.path_align), (PathEffect::Skew, PathAlign::Baseline));
        assert_eq!((b.path_effect, b.path_align), (PathEffect::Rainbow, PathAlign::Ascender));
        let (from, to) = a.kind.path_span().unwrap();
        assert!((to - from - 0.5).abs() < 1e-3, "{from} {to}");
    }

    /// Round a closed path the file writes the end bracket unwrapped, past the last segment (the
    /// start plus the run): the type keeps that run, nearly the whole circle here, not the bit
    /// from the start to the path's end.
    #[test]
    fn type_on_a_closed_path_keeps_its_unwrapped_end() {
        let Some(FrameKind::OnPath { start, end, .. }) = on_path_frame(&circle(), [3.86426, 7.85426]) else { panic!("on a path") };
        assert!((start - 3.86426).abs() < 1e-9 && (end - 3.85426).abs() < 1e-9, "{start} {end}");
        let (t, span) = on_path("type", on_path_frame(&circle(), [3.86426, 7.85426]).unwrap());
        assert!((span - 0.9975).abs() < 1e-3, "{span}");
        assert_eq!(t.plain_text(), "type");
        // Wrapped the other way, or below zero, the brackets are the same places.
        for range in [[3.86426, 3.85426], [-0.13574, 3.85426]] {
            let (_, other) = on_path("type", on_path_frame(&circle(), range).unwrap());
            assert!((other - span).abs() < 1e-6, "{range:?}: {other}");
        }
        // Values that aren't numbers fall back to the start of the path, a full turn.
        let Some(FrameKind::OnPath { start, end, .. }) = on_path_frame(&circle(), [f64::NAN, f64::INFINITY]) else { panic!("on a path") };
        assert!(start.is_finite() && end.is_finite());
    }

    /// A range inside the loop is kept as it is, and a run of exactly the loop is a full turn,
    /// where a short line of type doesn't overflow.
    #[test]
    fn type_on_a_closed_path_keeps_a_range_inside_it_and_a_full_turn() {
        let (_, half) = on_path("type", on_path_frame(&circle(), [0.5, 2.5]).unwrap());
        assert!((half - 0.5).abs() < 1e-3, "{half}");
        let (t, full) = on_path("type", on_path_frame(&circle(), [1.0, 5.0]).unwrap());
        assert!((full - 1.0).abs() < 1e-9, "{full}");
        assert!(!vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t).overflow, "a short line fits a full turn");
    }

    /// An open path keeps clamping: a range past its end stops at it.
    #[test]
    fn type_on_an_open_path_clamps_its_range() {
        let open: Vec<[f64; 8]> = circle().into_iter().take(2).collect();
        let Some(FrameKind::OnPath { start, end, .. }) = on_path_frame(&open, [0.5, 7.0]) else { panic!("on a path") };
        assert_eq!((start, end), (0.5, 2.0));
        let (_, span) = on_path("type", on_path_frame(&open, [0.5, 7.0]).unwrap());
        assert!((span - 0.75).abs() < 1e-3, "{span}");
    }

    /// A point-type story of `text` whose lines start at the heights `ys` (the layout's own: where
    /// the paragraph spacing has put them), every paragraph styled `para`.
    fn story(text: &str, ys: &[f64], para: &Para) -> Story {
        let size = 13.0;
        let paras = text.split_inclusive('\r').map(|p| (p.chars().count(), para.clone())).collect();
        Story {
            text: text.to_string(),
            anchor: (0.0, 0.0),
            lines: ys.iter().map(|y| (0.0, *y)).collect(),
            frames: vec![FrameKind::Point([1.0, 0.0, 0.0, 1.0, 0.0, 0.0])],
            paras,
            runs: vec![(
                text.chars().count(),
                Style {
                    font: None,
                    size,
                    leading: None,
                    tracking: 0.0,
                    baseline_shift: 0.0,
                    h_scale: 1.0,
                    v_scale: 1.0,
                    fill: Some(Color::BLACK),
                    stroke: None,
                    fill_pattern: None,
                    stroke_pattern: None,
                    stroke_width: 1.0,
                    stroke_cap: LineCap::Butt,
                    stroke_join: LineJoin::Miter,
                    stroke_miter_limit: 10.0,
                    caps: 0,
                    tate_chu_yoko: false,
                    standard_vertical_roman: false,
                },
            )],
            path_spacing: 0.0,
            vertical: false,
            tabs_left_out: false.into(),
        }
    }

    /// A layout of glyphs `advances` wide round a circle of radius `radius` (y down), from its left,
    /// outside it (their feet towards its centre) or inside it, the length of the circle between
    /// their centres `fraction` of their advances' mean.
    fn round_layout(advances: &[f64], radius: f64, outside: bool, fraction: f64) -> Val {
        round_layout_with(advances, radius, outside, fraction, "", false)
    }

    /// The same with `extra` in the line's `G`, and baselines that stay level as 3D Ribbon's do when
    /// `level`.
    fn round_layout_with(advances: &[f64], radius: f64, outside: bool, fraction: f64, extra: &str, level: bool) -> Val {
        let sign = if outside { 1.0 } else { -1.0 };
        let (mut xs, mut ys, mut matrices, mut ends) = (vec![], vec![], vec![], vec![0.0]);
        let mut angle = std::f64::consts::PI;
        for (i, a) in advances.iter().enumerate() {
            if i > 0 {
                angle += sign * (advances[i - 1] + a) * 0.5 * fraction / radius;
            }
            // Outside, clockwise on the page (y down); inside, the other way round.
            let along = (-angle.sin() * sign, angle.cos() * sign);
            let centre = (radius * angle.cos(), radius * angle.sin());
            let baseline = if level { (1.0, 0.0) } else { along };
            xs.push(centre.0 - baseline.0 * a * 0.5);
            ys.push(centre.1 - baseline.1 * a * 0.5);
            matrices.extend([baseline.0, baseline.1, -along.1, along.0, 0.0, 0.0]);
            ends.push(ends[i] + a);
        }
        let nums = |v: &[f64]| v.iter().map(|n| format!("{n}")).collect::<Vec<_>>().join(" ");
        let doc = format!(
            "/0 << /99 /F /6 [ << /99 /L /6 [ << /99 /S /6 [ << /99 /G {extra} /21 << /1 [ {} ] /2 [ {} ] /3 [ {} ] /4 [ {} ] >> >> ] >> ] >> ] >>",
            nums(&xs),
            nums(&ys),
            nums(&matrices),
            nums(&ends)
        );
        read(doc.as_bytes()).unwrap().get("0").unwrap().clone()
    }

    /// The file lays type on a path out closer round the outside of a curve and further apart round
    /// the inside: the spacing that does that is read from its layout, as the text engine's model
    /// has it (length + turn × spacing = advance).
    #[test]
    fn the_layout_of_type_on_a_path_gives_its_spacing() {
        let advances = [20.0, 30.0, 10.0, 25.0, 20.0];
        // Outside a circle of radius 100, 10 above the path: the centres are 100/110 of the advances apart.
        let rainbow = PathEffect::Rainbow;
        let spacing = path_spacing(&round_layout(&advances, 100.0, true, 100.0 / 110.0), rainbow);
        assert!((spacing - 10.0).abs() < 1e-6, "{spacing}");
        // Inside it, 20 above the path: 100/80 of them.
        let spacing = path_spacing(&round_layout(&advances, 100.0, false, 100.0 / 80.0), rainbow);
        assert!((spacing - 20.0).abs() < 1e-6, "{spacing}");
        // At their advances: none.
        assert!(path_spacing(&round_layout(&advances, 100.0, true, 1.0), rainbow).abs() < 1e-6);
        // Corners (here a turn of about four radians from glyph to glyph) aren't taken for a curve.
        assert_eq!(path_spacing(&round_layout(&advances, 5.0, true, 1.0), rainbow), 0.0);
        // A straight line turns nowhere, and a layout without glyphs says nothing.
        assert_eq!(path_spacing(&round_layout(&advances, 1e12, true, 0.9), rainbow), 0.0);
        assert_eq!(path_spacing(&read(b"/99 /F").unwrap(), rainbow), 0.0);
    }

    /// Align to Path moves the baselines off the path, and the spacing is the one on the path. On a
    /// circle of radius 100 with spacing 10, the path's length between two centres is 100/110 of
    /// their advances. Baselines on a smaller or larger circle are that much closer or further.
    #[test]
    fn the_spacing_of_aligned_type_is_measured_on_its_path() {
        let advances = [20.0, 30.0, 10.0, 25.0, 20.0];
        let fit =
            |radius: f64, extra: &str| path_spacing(&round_layout_with(&advances, radius, true, radius / 110.0, extra, false), PathEffect::Rainbow);
        // 20 towards the feet: Ascender.
        let spacing = fit(80.0, "/0 << /0 [ 0 20 ] >>");
        assert!((spacing - 10.0).abs() < 1e-6, "{spacing}");
        // 8 away from them: Descender.
        let spacing = fit(108.0, "/0 << /0 [ 0 -8 ] >>");
        assert!((spacing - 10.0).abs() < 1e-6, "{spacing}");
        // A distance that can't be read counts as none.
        for bad in ["/0 3", "/0 << /0 (x) >>", "/0 << /0 [ 0 ] >>"] {
            let spacing = fit(100.0, bad);
            assert!((spacing - 10.0).abs() < 1e-6, "{bad}: {spacing}");
        }
        // Skew's feet are upright: its alignment moves the baselines without changing their
        // lengths, so the distance doesn't count.
        let layout = |extra: &str| round_layout_with(&advances, 100.0, true, 100.0 / 110.0, extra, false);
        let plain = path_spacing(&layout(""), PathEffect::Skew);
        let aligned = path_spacing(&layout("/0 << /0 [ 0 20 ] >>"), PathEffect::Skew);
        assert!((aligned - plain).abs() < 1e-9, "{plain} {aligned}");
        // 3D Ribbon's feet are square to the path, as Rainbow's are.
        let level = round_layout_with(&advances, 80.0, true, 80.0 / 110.0, "/0 << /0 [ 0 20 ] >>", true);
        let spacing = path_spacing(&level, PathEffect::Ribbon3d);
        assert!((spacing - 10.0).abs() < 1e-6, "{spacing}");
    }

    /// Skew's feet stay upright, so on a straight path at 30 degrees they are not square to it. The
    /// path turns nowhere, so glyphs set further apart than their advances give no spacing, going
    /// right or left.
    #[test]
    fn skewed_type_on_a_straight_tilted_path_has_no_spacing() {
        let advances = [20.0, 30.0, 10.0, 25.0, 20.0];
        for degrees in [30.0f64, 210.0] {
            let along = kurbo::Vec2::new(degrees.to_radians().cos(), degrees.to_radians().sin());
            let (mut at, mut glyphs) = (0.0, vec![]);
            for (i, a) in advances.iter().enumerate() {
                if i > 0 {
                    at += (advances[i - 1] + a) * 0.5 * 1.2;
                }
                glyphs.push((Point::ORIGIN + along * at, along));
            }
            let layout = upright_layout(&advances, &glyphs);
            for effect in [PathEffect::Skew, PathEffect::StairStep, PathEffect::Gravity] {
                let spacing = path_spacing(&layout, effect);
                assert!(spacing.abs() < 1e-9, "{degrees}, {effect:?}: {spacing}");
            }
        }
    }

    /// Round the bottom of a circle the path runs left, and Skew's upright feet point away from the
    /// side the path turns to: the turn is still measured as Rainbow's is. Outside a circle of
    /// radius 100, 10 above the path, the centres are 100/110 of the advances apart: spacing 10.
    #[test]
    fn skewed_type_running_left_round_a_curve_gives_its_spacing() {
        let advances = [20.0, 30.0, 10.0, 25.0, 20.0];
        let mut angle = std::f64::consts::FRAC_PI_2 - 0.2;
        let mut glyphs = vec![];
        for (i, a) in advances.iter().enumerate() {
            if i > 0 {
                angle += (advances[i - 1] + a) * 0.5 * (100.0 / 110.0) / 100.0;
            }
            glyphs.push((Point::new(100.0 * angle.cos(), 100.0 * angle.sin()), kurbo::Vec2::new(-angle.sin(), angle.cos())));
        }
        let spacing = path_spacing(&upright_layout(&advances, &glyphs), PathEffect::Skew);
        assert!((spacing - 10.0).abs() < 1e-6, "{spacing}");
    }

    /// A line's layout whose glyphs have their centres and baselines `glyphs` and upright feet, as
    /// Skew's are.
    fn upright_layout(advances: &[f64], glyphs: &[(Point, kurbo::Vec2)]) -> Val {
        let glyphs: Vec<(Point, [f64; 4])> = glyphs.iter().map(|(centre, along)| (*centre, [along.x, along.y, 0.0, 1.0])).collect();
        line_layout(advances, &glyphs, "")
    }

    /// A line's layout, `extra` in its `G`, whose glyphs have their centres and matrices `glyphs`.
    fn line_layout(advances: &[f64], glyphs: &[(Point, [f64; 4])], extra: &str) -> Val {
        let (mut xs, mut ys, mut matrices, mut ends) = (vec![], vec![], vec![], vec![0.0]);
        for (i, (a, (centre, [m0, m1, m2, m3]))) in advances.iter().zip(glyphs).enumerate() {
            xs.push(centre.x - m0 * a * 0.5);
            ys.push(centre.y - m1 * a * 0.5);
            matrices.extend([*m0, *m1, *m2, *m3, 0.0, 0.0]);
            ends.push(ends[i] + a);
        }
        let nums = |v: &[f64]| v.iter().map(|n| format!("{n}")).collect::<Vec<_>>().join(" ");
        let doc = format!(
            "/0 << /99 /F /6 [ << /99 /L /6 [ << /99 /S /6 [ << /99 /G {extra} /21 << /1 [ {} ] /2 [ {} ] /3 [ {} ] /4 [ {} ] >> >> ] >> ] >> ] >>",
            nums(&xs),
            nums(&ys),
            nums(&matrices),
            nums(&ends)
        );
        read(doc.as_bytes()).unwrap().get("0").unwrap().clone()
    }

    /// 3D Ribbon keeps its baselines level, so its glyphs' feet show how the path turns.
    #[test]
    fn the_spacing_of_3d_ribbon_type_follows_the_feet_of_its_glyphs() {
        let advances = [20.0, 30.0, 10.0, 25.0, 20.0];
        let layout = round_layout_with(&advances, 100.0, true, 100.0 / 110.0, "", true);
        let spacing = path_spacing(&layout, PathEffect::Ribbon3d);
        assert!((spacing - 10.0).abs() < 1e-6, "{spacing}");
    }

    /// Where the path runs left, 3D Ribbon's level baseline faces left and its foot turns round with
    /// it. Round a circle of radius 100 through the place where the path runs straight up, the glyphs
    /// on either side of it are still read as following one curve: spacing 10.
    #[test]
    fn the_spacing_of_3d_ribbon_type_holds_where_the_path_turns_back() {
        let advances = [20.0, 30.0];
        let mut angle = std::f64::consts::PI - 0.1;
        let mut glyphs = vec![];
        for (i, a) in advances.iter().enumerate() {
            if i > 0 {
                angle += (advances[i - 1] + a) * 0.5 * (100.0 / 110.0) / 100.0;
            }
            let along = kurbo::Vec2::new(-angle.sin(), angle.cos());
            let side = if along.x < 0.0 { -1.0 } else { 1.0 };
            glyphs.push((Point::new(100.0 * angle.cos(), 100.0 * angle.sin()), [side, 0.0, -along.y * side, along.x * side]));
        }
        assert!(glyphs.first().is_some_and(|(_, m)| m[0] < 0.0) && glyphs.last().is_some_and(|(_, m)| m[0] > 0.0));
        let spacing = path_spacing(&line_layout(&advances, &glyphs, ""), PathEffect::Ribbon3d);
        assert!((spacing - 10.0).abs() < 1e-6, "{spacing}");
    }

    /// Stair Step's glyphs stay upright, each origin on the path, its step the path's length to the
    /// next. On a circle of radius 100 with spacing 10, each step is 100/110 of the glyph's advance;
    /// moved 20 down by an alignment, the origins give the same spacing.
    #[test]
    fn the_spacing_of_stair_step_type_follows_the_origins_of_its_glyphs() {
        let advances = [20.0, 30.0, 10.0, 25.0, 20.0];
        for rise in [0.0, 20.0] {
            let mut s = 0.0;
            let mut glyphs = vec![];
            for a in advances {
                let angle = std::f64::consts::PI + s / 100.0;
                // `line_layout` takes centres: half an advance along the level baseline.
                glyphs.push((Point::new(100.0 * angle.cos() + a * 0.5, 100.0 * angle.sin() + rise), [1.0, 0.0, 0.0, 1.0]));
                s += a * 100.0 / 110.0;
            }
            let spacing = path_spacing(&line_layout(&advances, &glyphs, ""), PathEffect::StairStep);
            assert!((spacing - 10.0).abs() < 1e-6, "{rise}: {spacing}");
        }
        // Two glyphs say nothing of how the path turns.
        let two = line_layout(&advances[..2], &[(Point::new(0.0, 0.0), [1.0, 0.0, 0.0, 1.0]), (Point::new(30.0, 5.0), [1.0, 0.0, 0.0, 1.0])], "");
        assert_eq!(path_spacing(&two, PathEffect::StairStep), 0.0);
    }

    /// With Ascender (baselines 20 towards the feet), glyphs facing left lie outside the path and
    /// glyphs facing right inside it. The pair either side of where the path runs straight up is left
    /// out, and the others give spacing 10.
    #[test]
    fn aligned_3d_ribbon_type_leaves_out_the_pair_where_the_path_turns_back() {
        let advances = [20.0, 30.0, 10.0, 25.0, 20.0];
        let mut angle = std::f64::consts::PI - 0.5;
        let mut glyphs = vec![];
        for (i, a) in advances.iter().enumerate() {
            if i > 0 {
                angle += (advances[i - 1] + a) * 0.5 / 110.0;
            }
            let along = kurbo::Vec2::new(-angle.sin(), angle.cos());
            let side = if along.x < 0.0 { -1.0 } else { 1.0 };
            let radius = 100.0 - 20.0 * side;
            glyphs.push((Point::new(radius * angle.cos(), radius * angle.sin()), [side, 0.0, -along.y * side, along.x * side]));
        }
        let sides: Vec<f64> = glyphs.iter().map(|(_, m)| m[0]).collect();
        assert_eq!(sides, [-1.0, -1.0, -1.0, 1.0, 1.0]);
        let spacing = path_spacing(&line_layout(&advances, &glyphs, "/0 << /0 [ 0 20 ] >>"), PathEffect::Ribbon3d);
        assert!((spacing - 10.0).abs() < 1e-6, "{spacing}");
    }

    /// The leading of the text object a story comes in as.
    fn leading_of(story: &Story) -> Option<f64> {
        let node = story.node(NodeId(1), 0, (0.0, 0.0), Affine::IDENTITY)?;
        match &node.kind {
            NodeKind::Text(t) => t.runs.first().and_then(|r| r.style.leading),
            _ => None,
        }
    }

    /// The baselines of the text object `story` comes in as, laid out.
    fn baselines_of(story: &Story) -> Vec<f64> {
        let Some(node) = story.node(NodeId(1), 0, (0.0, 0.0), Affine::IDENTITY) else { return vec![] };
        let NodeKind::Text(t) = &node.kind else { return vec![] };
        vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t).baselines().iter().map(|b| b.0.y).collect()
    }

    /// Auto leading stays auto: each line takes the paragraph's share of the largest type on it, as
    /// the file's own layout does, not the distance between the first two lines for every line. A
    /// 15 pt heading over 10 pt type at 120%: 18 pt between the heading's lines, then 12 pt.
    #[test]
    fn auto_leading_follows_the_size_of_each_line() {
        let mut s = story("Big\x03Bigger\rsmall\r", &[0.0, 18.0, 30.0], &Para::default());
        let base = s.runs[0].1.clone();
        s.runs = vec![(11, Style { size: 15.0, ..base.clone() }), (6, Style { size: 10.0, ..base })];
        assert_eq!(leading_of(&s), None);
        let ys = baselines_of(&s);
        assert_eq!(ys.len(), 3, "{ys:?}");
        assert!(((ys[1] - ys[0]).abs() - 18.0).abs() < 1e-6, "{ys:?}");
        assert!(((ys[2] - ys[1]).abs() - 12.0).abs() < 1e-6, "{ys:?}");
        // At a share the paragraph sets (key 7) of 150%: 22.5 pt, then 15 pt.
        let share = Para { auto_leading: Some(1.5), ..Para::default() };
        s.paras = s.paras.iter().map(|(n, _)| (*n, share.clone())).collect();
        let ys = baselines_of(&s);
        assert!(((ys[1] - ys[0]).abs() - 22.5).abs() < 1e-6, "{ys:?}");
        assert!(((ys[2] - ys[1]).abs() - 15.0).abs() < 1e-6, "{ys:?}");
    }

    #[test]
    fn leading_leaves_out_the_space_between_paragraphs_that_the_lines_already_have() {
        // Lines 10.6 apart that are three paragraphs with 5 less after each: auto leading (15.6) less 5.
        let para = Para { space_after: -5.0, ..Para::default() };
        // The text object keeps auto leading and the spacing, so its lines are where the file's are.
        let s = story("one\rtwo\rthree\r", &[0.0, 10.6, 21.2], &para);
        assert_eq!(leading_of(&s), None);
        let ys = baselines_of(&s);
        assert!(ys.len() == 3 && ys.windows(2).all(|w| ((w[1] - w[0]).abs() - 10.6).abs() < 1e-6), "{ys:?}");
        // Space after and space before both count.
        let para = Para { space_after: 4.0, space_before: 2.0, ..Para::default() };
        let ys = baselines_of(&story("one\rtwo\r", &[0.0, 21.6], &para));
        assert!(((ys[1] - ys[0]).abs() - 21.6).abs() < 1e-6, "{ys:?}");
    }

    /// #2031: a Large Canvas editing copy measures its text document in its art's units, a tenth
    /// of the page's: the story's size and leading reach the page through `to_doc`'s scale.
    #[test]
    fn a_story_in_a_scaled_art_space_keeps_its_size_and_leading() {
        let s = story("one\rtwo\r", &[0.0, 15.6], &Para::default());
        // A copy written at 1:10 (see `ai::read::ensure_space`): its art's units reach the
        // document, 100 points to the page, through `to_doc`'s scale.
        let to_doc = Affine::scale(10.0) * Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 100.0]);
        let t = match s.node(NodeId(1), 0, (0.0, 0.0), to_doc).unwrap().kind {
            NodeKind::Text(t) => t,
            _ => panic!("not text"),
        };
        assert_eq!(t.runs[0].style.size, 130.0, "13 pt at the page's scale");
        assert_eq!(t.runs[0].style.leading, None, "auto leading: 120% of its size at the page's scale");
        assert_eq!(t.para.justification.auto_leading, 120.0);
    }

    #[test]
    fn leading_between_lines_of_a_paragraph_is_their_distance() {
        // A line break inside a paragraph has no paragraph spacing between its lines.
        let para = Para { space_after: -5.0, space_before: 3.0, ..Para::default() };
        let ys = baselines_of(&story("one\x03two\x03three\r", &[0.0, 15.6, 31.2], &para));
        assert!(ys.len() == 3 && ys.windows(2).all(|w| ((w[1] - w[0]).abs() - 15.6).abs() < 1e-6), "{ys:?}");
        // One line.
        assert_eq!(baselines_of(&story("one\r", &[0.0], &Para::default())).len(), 1);
    }

    /// A `\x03` ends a line inside its paragraph: it comes in as a forced line break, and the
    /// paragraph's style runs over it to the paragraph's end.
    #[test]
    fn a_soft_return_is_a_line_break_in_its_paragraph() {
        let para = Para { space_before: 4.0, space_after: 2.0, first_indent: 6.0, ..Para::default() };
        let mut s = story("A\x03B\rC\r", &[0.0, 15.6, 31.2], &para);
        s.paras[1].1 = Para { left_indent: 3.0, ..para.clone() };
        let node = s.node(NodeId(1), 0, (0.0, 0.0), Affine::IDENTITY).unwrap();
        let NodeKind::Text(t) = &node.kind else { panic!("not text") };
        assert_eq!(t.plain_text(), "A\u{2028}B\nC");
        assert_eq!(t.paras.len(), 2, "{:?}", t.paras);
        assert_eq!((t.paras[0].first_line_indent, t.paras[0].left_indent, t.paras[0].space_before), (6.0, 0.0, 4.0));
        assert_eq!((t.paras[1].first_line_indent, t.paras[1].left_indent), (6.0, 3.0));
    }

    #[test]
    fn a_frame_too_short_for_its_first_line_is_made_taller() {
        // A heading in a frame as tall as its size: a line of the font it shows in here is taller.
        let s = story("Heading\r", &[0.0], &Para::default());
        let area = |height: f64| {
            let mut t = TextObject::point(Point::ZERO, "Heading", CharStyle { size: 13.0, ..CharStyle::default() });
            t.kind = TextKind::Area { frame: vectorcraft_geom::shapes::rectangle(Rect::new(10.0, 20.0, 210.0, 20.0 + height)) };
            t
        };
        let height =
            |t: &TextObject| if let TextKind::Area { frame } = &t.kind { frame.bounds().map(|b| (b.x0, b.y0, b.x1, b.height())) } else { None };
        let lines = |t: &TextObject| vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t).lines.len();
        let mut t = area(13.0);
        assert_eq!(lines(&t), 0, "nothing of it shows");
        assert!(s.fit_first_line(&mut t));
        let (x0, y0, x1, h) = height(&t).unwrap();
        assert!((x0, y0, x1) == (10.0, 20.0, 210.0) && h > 13.0 && h < 26.0, "taller from its top: {h}");
        assert_eq!(lines(&t), 1);
        // A frame that holds the line stays as it is; so does one the file lays no line out in
        // (its type is overset in the file too).
        let mut t = area(40.0);
        assert!(!s.fit_first_line(&mut t) && height(&t).unwrap().3 == 40.0);
        let mut t = area(13.0);
        assert!(!story("Heading\r", &[], &Para::default()).fit_first_line(&mut t) && height(&t).unwrap().3 == 13.0);
    }

    /// The file sets three lines in a frame whose third line's descent passes its bottom: the frame
    /// grows from its bottom so the third shows. Only when the story's one frame is area type.
    #[test]
    fn the_last_lines_the_file_sets_in_its_frame_show() {
        let db = vectorcraft_text::FontDb::global();
        let area = |height: f64| {
            let mut t = TextObject::point(Point::ZERO, "one\ntwo\nthree", CharStyle { size: 13.0, ..CharStyle::default() });
            t.kind = TextKind::Area { frame: vectorcraft_geom::shapes::rectangle(Rect::new(10.0, 20.0, 210.0, 20.0 + height)) };
            t
        };
        let tall = vectorcraft_text::layout(db, &area(400.0));
        let third = &tall.lines[2];
        let mut t = area(third.baseline - 20.0 + third.descent / 2.0);
        assert_eq!(vectorcraft_text::layout(db, &t).lines.len(), 2);
        let mut s = story("one\rtwo\rthree\r", &[0.0, 15.6, 31.2], &Para::default());
        // A point-type frame (the helper's) isn't fitted.
        assert!(!s.fit_last_lines(&mut t));
        s.frames = vec![FrameKind::Area(vec![], AreaFrame::default())];
        assert!(s.fit_last_lines(&mut t));
        assert_eq!(vectorcraft_text::layout(db, &t).lines.len(), 3);
        // A story threaded through two frames: its lines are the first frame's alone.
        let mut t = area(third.baseline - 20.0 + third.descent / 2.0);
        s.frames.push(FrameKind::Area(vec![], AreaFrame::default()));
        assert!(!s.fit_last_lines(&mut t));
    }

    #[test]
    fn area_type_s_first_line_goes_where_the_file_lays_it_out() {
        // GUARDS: area type's first baseline was the font's ascent below the frame's top, where the
        // file's own layout (and its page) can have it higher: a heading came in lower than the page's.
        let line = |a: (f64, f64), b: (f64, f64)| [a.0, a.1, a.0, a.1, b.0, b.1, b.0, b.1];
        let frame = vec![
            line((10.0, 20.0), (210.0, 20.0)),
            line((210.0, 20.0), (210.0, 120.0)),
            line((210.0, 120.0), (10.0, 120.0)),
            line((10.0, 120.0), (10.0, 20.0)),
        ];
        // The canvas is the document here: the art's y up, the document's down.
        let (template, to_doc) = ((CANVAS_CENTRE, -CANVAS_CENTRE), Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 0.0]));
        let first = |s: &Story| {
            let mut node = s.node(NodeId(0), 0, template, to_doc).unwrap();
            let placed = s.place_first_line(&mut node, 0, template, to_doc);
            let NodeKind::Text(t) = node.kind else { panic!("area type") };
            let l = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
            (placed, l.lines.first().map(|l| l.baseline), t.area.first_baseline)
        };
        let mut s = story("Heading\r", &[], &Para::default());
        s.frames = vec![FrameKind::Area(frame, AreaFrame::default())];
        // Where this lays the line out on its own: the file agrees, and nothing changes.
        s.lines = vec![(10.0, 20.0)];
        let (_, Some(own), _) = first(&s) else { panic!("a line") };
        s.lines = vec![(10.0, own)];
        assert_eq!(first(&s), (false, Some(own), vectorcraft_doc::FirstBaseline::Ascent));
        // Higher in the file: there.
        s.lines = vec![(10.0, own - 3.0)];
        let (placed, at, fb) = first(&s);
        assert!(placed && fb == vectorcraft_doc::FirstBaseline::Fixed, "{fb:?}");
        assert!(at.is_some_and(|y| (y - (own - 3.0)).abs() < 1e-6), "{at:?} for {}", own - 3.0);
    }

    /// A text document's area frame, the rectangle 10, 20 to 210, 120 of the canvas, whose frame
    /// data has `options` besides its kind, written by hand.
    fn area_frame(options: &str) -> FrameKind {
        let line = |a: (f64, f64), b: (f64, f64)| format!("{} {} {} {} {} {} {} {}", a.0, a.1, a.0, a.1, b.0, b.1, b.0, b.1);
        let segments = [
            line((10.0, 20.0), (210.0, 20.0)),
            line((210.0, 20.0), (210.0, 120.0)),
            line((210.0, 120.0), (10.0, 120.0)),
            line((10.0, 120.0), (10.0, 20.0)),
        ];
        let doc = format!("/0 << /1 << /0 [ {} ] >> /2 << /0 1 {options} >> >>", segments.join(" "));
        let texts = Texts {
            stories: vec![],
            fonts: vec![],
            defaults: Val::Dict(BTreeMap::new()),
            para_defaults: Val::Dict(BTreeMap::new()),
            char_sheets: vec![],
            para_sheets: vec![],
            frames: vec![read(doc.as_bytes()).unwrap()],
            patterns: HashMap::new(),
        };
        texts.frame(0).unwrap()
    }

    /// The area options of the text object a story of `frames` comes in as at frame `frame`, the
    /// art's units `scale` document points each.
    fn area_of(frames: Vec<FrameKind>, frame: usize, scale: f64) -> vectorcraft_doc::AreaOptions {
        let mut s = story("Some area type\r", &[], &Para::default());
        s.frames = frames;
        let to_doc = Affine::scale(scale) * Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 0.0]);
        let NodeKind::Text(t) = s.node(NodeId(0), frame, (CANVAS_CENTRE, -CANVAS_CENTRE), to_doc).unwrap().kind else { panic!("area type") };
        t.area
    }

    #[test]
    fn area_frame_options_are_read() {
        // GUARDS: area type came in with one row, one column and the 18 pt gutter whatever its frame kept.
        let a = area_of(vec![area_frame("/3 2 /4 3 /7 20 /8 24")], 0, 1.0);
        assert_eq!((a.rows, a.columns, a.gutter, a.row_gutter), (2, 3, 24.0, Some(20.0)));
        // The same gutter between rows and columns is one gutter.
        let a = area_of(vec![area_frame("/3 2 /4 3 /7 20 /8 20")], 0, 1.0);
        assert_eq!((a.gutter, a.row_gutter), (20.0, None));
        // Lengths scale with the art's units (a Large Canvas copy measures in tenths of the page).
        let a = area_of(vec![area_frame("/7 2 /8 2.4 /10 << /0 3 /1 3 >>")], 0, 10.0);
        assert!((a.gutter - 24.0).abs() < 1e-9 && a.row_gutter == Some(20.0), "{a:?}");
        assert!((a.first_baseline_min - 30.0).abs() < 1e-9, "{a:?}");
    }

    #[test]
    fn area_frame_first_baseline_kinds_and_minimum_are_read() {
        use vectorcraft_doc::FirstBaseline as F;
        for (code, kind) in [(0, F::Fixed), (2, F::CapHeight), (3, F::Leading), (4, F::XHeight)] {
            let a = area_of(vec![area_frame(&format!("/10 << /0 {code} /1 30 >>"))], 0, 2.0);
            assert_eq!((a.first_baseline, a.first_baseline_min), (kind, 60.0), "code {code}");
        }
        // A kind without its own first baseline here, or none known, leaves Ascent.
        for code in ["1", "5", "6", "7", "-1", "2.5", "(x)"] {
            let a = area_of(vec![area_frame(&format!("/10 << /0 {code} /1 30 >>"))], 0, 1.0);
            assert_eq!((a.first_baseline, a.first_baseline_min), (F::Ascent, 0.0), "code {code}");
        }
        // A minimum that can't be read is none.
        let a = area_of(vec![area_frame("/10 << /0 3 /1 -4 >>")], 0, 1.0);
        assert_eq!((a.first_baseline, a.first_baseline_min), (F::Leading, 0.0));
    }

    /// A first baseline the frame keeps stays as it is: the first line isn't moved to where the
    /// file lays it out.
    #[test]
    fn a_first_baseline_the_frame_keeps_is_not_moved() {
        let (template, to_doc) = ((CANVAS_CENTRE, -CANVAS_CENTRE), Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 0.0]));
        // Where this lays the first line out on its own.
        let mut s = story("Heading\r", &[], &Para::default());
        s.frames = vec![area_frame("")];
        let NodeKind::Text(t) = s.node(NodeId(0), 0, template, to_doc).unwrap().kind else { panic!("area type") };
        let own = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t).lines.first().map(|l| l.baseline).unwrap();
        let placed_in = |frames: &[&str]| {
            let mut s = story("Heading\r", &[], &Para::default());
            s.frames = frames.iter().map(|o| area_frame(o)).collect();
            // Higher in the file.
            s.lines = vec![(10.0, own - 3.0)];
            let mut node = s.node(NodeId(0), 0, template, to_doc).unwrap();
            let placed = s.place_first_line(&mut node, 0, template, to_doc);
            let NodeKind::Text(t) = node.kind else { panic!("area type") };
            (placed, t.area.first_baseline)
        };
        let placed = |options: &str| placed_in(&[options]);
        assert_eq!(placed("/10 << /0 2 /1 0 >>"), (false, vectorcraft_doc::FirstBaseline::CapHeight));
        // Without one (Ascent), or with one that isn't read, the line goes where the file has it.
        assert_eq!(placed(""), (true, vectorcraft_doc::FirstBaseline::Fixed));
        assert_eq!(placed("/10 << /0 5 /1 0 >>"), (true, vectorcraft_doc::FirstBaseline::Fixed));
        // Another frame of the thread keeping one doesn't count for the first.
        assert_eq!(placed_in(&["", "/10 << /0 2 /1 0 >>"]), (true, vectorcraft_doc::FirstBaseline::Fixed));
    }

    #[test]
    fn area_frame_options_that_cannot_be_read_keep_their_defaults() {
        let defaults = vectorcraft_doc::AreaOptions::default();
        let bad = [
            "/3 0 /4 2.5 /7 -1 /8 (x)",
            "/3 NaN /4 inf /7 NaN /8 -inf /10 [ 1 2 ]",
            "/3 (2) /4 [ 3 ] /7 << /0 1 >> /8 true /10 3",
            "/10 << /1 30 >>",
        ];
        for options in bad {
            assert_eq!(area_of(vec![area_frame(options)], 0, 1.0), defaults, "{options}");
        }
        // Too many rows and columns, and too long gutters, are capped.
        let a = area_of(vec![area_frame("/3 1e9 /4 101 /7 1e300 /8 5e4")], 0, 1.0);
        assert_eq!((a.rows, a.columns, a.gutter, a.row_gutter), (100, 100, MAX_AREA_LENGTH, None));
        // The cap holds after the lengths scale with the art's units.
        let a = area_of(vec![area_frame("/7 1e4 /8 2e3 /10 << /0 0 /1 1e4 >>")], 0, 10.0);
        assert_eq!((a.gutter, a.row_gutter, a.first_baseline_min), (MAX_AREA_LENGTH, None, MAX_AREA_LENGTH));
        // One that can be read is read beside one that can't.
        let a = area_of(vec![area_frame("/3 2 /4 -3 /8 12 /7 NaN")], 0, 1.0);
        assert_eq!((a.rows, a.columns, a.gutter, a.row_gutter), (2, 1, 12.0, Some(18.0)));
    }

    #[test]
    fn area_frame_without_options_comes_in_as_before() {
        assert_eq!(area_of(vec![area_frame("")], 0, 1.0), vectorcraft_doc::AreaOptions::default());
        assert_eq!(area_frame(""), FrameKind::Area(area_segments(), AreaFrame::default()));
    }

    /// The segments [`area_frame`] writes.
    fn area_segments() -> Vec<[f64; 8]> {
        let line = |a: (f64, f64), b: (f64, f64)| [a.0, a.1, a.0, a.1, b.0, b.1, b.0, b.1];
        vec![
            line((10.0, 20.0), (210.0, 20.0)),
            line((210.0, 20.0), (210.0, 120.0)),
            line((210.0, 120.0), (10.0, 120.0)),
            line((10.0, 120.0), (10.0, 20.0)),
        ]
    }

    #[test]
    fn each_frame_of_a_thread_keeps_its_own_options() {
        let frames = vec![area_frame("/3 2 /7 10 /8 10 /10 << /0 0 /1 12 >>"), area_frame("/4 3 /7 6 /8 8")];
        let (first, second) = (area_of(frames.clone(), 0, 1.0), area_of(frames, 1, 1.0));
        assert_eq!((first.rows, first.columns, first.gutter, first.row_gutter), (2, 1, 10.0, None));
        assert_eq!((first.first_baseline, first.first_baseline_min), (vectorcraft_doc::FirstBaseline::Fixed, 12.0));
        assert_eq!((second.rows, second.columns, second.gutter, second.row_gutter), (1, 3, 8.0, Some(6.0)));
        assert_eq!(second.first_baseline, vectorcraft_doc::FirstBaseline::Ascent);
    }

    /// Vertical area type: the file's first line is its first column's centre line, so far left of
    /// the frame's right edge; the column goes there.
    #[test]
    fn vertical_area_type_s_first_column_goes_where_the_file_lays_it_out() {
        let line = |a: (f64, f64), b: (f64, f64)| [a.0, a.1, a.0, a.1, b.0, b.1, b.0, b.1];
        let frame = vec![
            line((10.0, 20.0), (210.0, 20.0)),
            line((210.0, 20.0), (210.0, 120.0)),
            line((210.0, 120.0), (10.0, 120.0)),
            line((10.0, 120.0), (10.0, 20.0)),
        ];
        let (template, to_doc) = ((CANVAS_CENTRE, -CANVAS_CENTRE), Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 0.0]));
        // Whether the first line was placed, the first column's centre line (its x on the document)
        // and the first baseline setting.
        let first = |s: &Story, place: bool| {
            let mut node = s.node(NodeId(0), 0, template, to_doc).unwrap();
            let placed = place && s.place_first_line(&mut node, 0, template, to_doc);
            let NodeKind::Text(t) = node.kind else { panic!("area type") };
            assert!(t.vertical);
            let l = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
            (placed, l.baselines().first().map(|b| b.0.x), t.area.first_baseline)
        };
        let mut s = story("§§\r", &[], &Para::default());
        s.frames = vec![FrameKind::Area(frame, AreaFrame::default())];
        s.vertical = true;
        s.anchor = (210.0, 20.0);
        s.lines = vec![(0.0, 0.0)];
        let (_, Some(own), _) = first(&s, false) else { panic!("a column") };
        assert!(own < 210.0 && own > 190.0, "near the right edge: {own}");
        // The file's column where this lays it out on its own (so far left of the anchor, the
        // frame's top right): nothing changes.
        s.lines = vec![(0.0, 210.0 - own)];
        assert_eq!(first(&s, true), (false, Some(own), vectorcraft_doc::FirstBaseline::Ascent));
        // Further right in the file: there.
        s.lines = vec![(0.0, 210.0 - own - 3.0)];
        let (placed, at, fb) = first(&s, true);
        assert!(placed && fb == vectorcraft_doc::FirstBaseline::Fixed, "{fb:?}");
        assert!(at.is_some_and(|x| (x - (own + 3.0)).abs() < 1e-6), "{at:?} for {}", own + 3.0);
        // Scaled characters (a column of another width) go there too.
        s.runs[0].1.v_scale = 2.0;
        s.runs[0].1.h_scale = 2.0;
        s.lines = vec![(0.0, 0.0)];
        let (_, Some(own), _) = first(&s, false) else { panic!("a column") };
        s.lines = vec![(0.0, 210.0 - own - 3.0)];
        let (placed, at, _) = first(&s, true);
        assert!(placed && at.is_some_and(|x| (x - (own + 3.0)).abs() < 1e-6), "{at:?} for {}", own + 3.0);
    }
}
