//! The text of an Illustrator editing copy.
//!
//! The editing copy of a file keeps the characters of its text objects, with their fonts, sizes,
//! colours and where they sit, in one text document of its own: between `%AI11_BeginTextDocument`
//! and `%AI11_EndTextDocument`, ASCII85 of nested dictionaries with numbers for keys. A text object
//! in a layer is only a stub naming its story in it (`/StoryIndex`).
//!
//! The format isn't documented. What is read here was worked out from the files of the project's
//! users, used locally and never committed, by comparing it with the type their pages draw: the
//! meaning of each key below was checked against the page's own type in more than a thousand text
//! objects, and it is only used for the keys that matched. A story the reader doesn't recognise (area
//! type, type on a path, anything with a frame that has more than a point and a matrix) is left
//! alone: it has no text, and the importer says so.
//!
//! Where type sits: a story lays its lines out round a point `F` (the anchor: the start of a line of
//! left-aligned type, the middle of a centred one, the end of a right-aligned one), each line with an
//! offset from it. A right-to-left paragraph (key `33` of its attributes) swaps the two: its
//! "left"-aligned lines end at the anchor and its "right"-aligned ones start there. The frame of the
//! story gives a matrix that carries that layout onto the canvas of the app, which is 16383 points a
//! side; the canvas's centre, 8191.5, is the centre of the file's `%AI3_TemplateBox`. See
//! [`Story::place`].

use std::collections::BTreeMap;

use kurbo::{BezPath, CubicBez, ParamCurve, ParamCurveArclen};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{CharStyle, Justify, Node, NodeId, NodeKind, ParaStyle, TextKind, TextObject, TextRun};
use vectorcraft_geom::{Affine, PathData, Point, Rect};

/// Longest story read (bytes).
const MAX_STORY: usize = 1 << 20;
/// Biggest text document read (bytes, decoded).
const MAX_BYTES: usize = 64 << 20;
/// Deepest nesting read.
const MAX_DEPTH: usize = 64;
/// Most values read.
const MAX_VALUES: usize = 4_000_000;
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

/// Most frames read of one story, and most path segments of a frame.
const MAX_FRAMES: usize = 4096;
const MAX_SEGMENTS: usize = 1 << 16;

/// Where a frame puts its story's text.
#[derive(Debug, Clone)]
enum FrameKind {
    /// Point type: the matrix carries the layout (its anchor at the origin of its lines) onto the
    /// canvas.
    Point([f64; 6]),
    /// Area type flowed inside a path: its cubic segments on the canvas, four points each.
    Area(Vec<[f64; 8]>),
    /// Type on a path, from `start` to `end` (segment index and the place in that segment).
    OnPath { segments: Vec<[f64; 8]>, start: f64, end: f64 },
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
}

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
    stroke_width: f64,
}

/// The text document of an editing copy.
pub(super) struct Texts {
    stories: Vec<Val>,
    fonts: Vec<String>,
    /// The character and paragraph styles a story starts from.
    defaults: Val,
    para_defaults: Val,
    frames: Vec<Val>,
}

impl Texts {
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
        Some(Texts {
            stories: main.get("1")?.list().to_vec(),
            fonts,
            defaults: main.get("2")?.clone(),
            para_defaults: main.get("3").cloned().unwrap_or(Val::Dict(BTreeMap::new())),
            frames: doc.get("0")?.get("8")?.get("0")?.list().to_vec(),
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
            1 => Some(FrameKind::Area(segments()?)),
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
                Some(FrameKind::OnPath { segments, start, end })
            }
            _ => None,
        }
    }

    /// Story `i`, if its frames are of kinds this reads.
    pub(super) fn story(&self, i: usize) -> Option<Story> {
        let s = self.stories.get(i)?;
        let text = s.get("0")?.get("0")?.str().filter(|t| t.len() <= MAX_STORY)?.to_string();
        let view = s.get("1")?;
        let frames: Vec<FrameKind> = view
            .get("0")?
            .list()
            .iter()
            .take(MAX_FRAMES)
            .map(|f| f.get("0").and_then(Val::num).and_then(|n| self.frame(n as usize)))
            .collect::<Option<_>>()?;
        if frames.is_empty() {
            return None;
        }
        // The first frame's layout, when the file keeps one: where its lines start.
        let (mut anchor, mut lines) = ((0.0, 0.0), vec![]);
        if let Some(f) = view.get("2").and_then(|l| find_node(l, "F")) {
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
            let props = run.get("0").and_then(|r| r.get("0")).and_then(|r| r.get("5"));
            paras.push((n, self.para(props)));
        }
        let mut runs = vec![];
        for run in s.get("0")?.get("6").map(|p| p.get("0").map(Val::list).unwrap_or_default()).unwrap_or_default() {
            let n = run.get("1").and_then(Val::num)? as usize;
            let props = run.get("0").and_then(|r| r.get("0")).and_then(|r| r.get("6"));
            runs.push((n, self.style(props)));
        }
        if runs.is_empty() {
            runs.push((text.chars().count(), self.style(None)));
        }
        Some(Story { text, anchor, lines, frames, paras, runs })
    }

    /// A paragraph's attributes: its own keys, else the document's.
    fn para(&self, p: Option<&Val>) -> Para {
        let pick = |k: &str| p.and_then(|p| p.get(k)).or_else(|| self.para_defaults.get(k));
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
        }
    }

    /// A character style: its own keys, else the document's.
    fn style(&self, s: Option<&Val>) -> Style {
        let pick = |k: &str| s.and_then(|s| s.get(k)).or_else(|| self.defaults.get(k));
        let num = |k: &str| pick(k).and_then(Val::num);
        let on = |k: &str| !matches!(pick(k), Some(Val::Bool(false)));
        let font = num("0").and_then(|i| self.fonts.get(i as usize)).filter(|f| !f.is_empty()).cloned();
        let size = num("1").filter(|v| *v > 0.0 && *v < 1e5).unwrap_or(12.0);
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
            stroke_width: num("63").filter(|v| *v >= 0.0 && *v < 1e4).unwrap_or(1.0),
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

impl Story {
    /// How many bytes of text the story has.
    pub(super) fn len(&self) -> usize {
        self.text.len()
    }

    /// The story's characters, with `\r` ending paragraphs and `\x03` ending lines.
    pub(super) fn text(&self) -> &str {
        &self.text
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
                let (x, y) = (self.anchor.0 + lx, self.anchor.1 + ly);
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
            FrameKind::Area(s) => bounds(s)?,
            FrameKind::OnPath { segments, .. } => bounds(segments)?.inflate(size * 1.5, size * 1.5),
            FrameKind::Point(_) => {
                let longest = self.text.split(['\r', '\x03']).map(|l| l.chars().count()).max().unwrap_or(0) as f64;
                let reach = longest * size + size;
                let points: Vec<Point> = self.line_starts(template).into_iter().map(|(_, (x, y))| to_doc * Point::new(x, y)).collect();
                let first = points.first().copied().unwrap_or_else(|| {
                    let (x, y) = self.place(template);
                    to_doc * Point::new(x, y)
                });
                points.iter().fold(Rect::from_points(first, first), |r, p| r.union_pt(*p)).inflate(reach, size * 1.5)
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
            FrameKind::Area(s) => (1.0, Affine::IDENTITY, TextKind::Area { frame: path_of(s, template, to_doc) }),
            FrameKind::OnPath { segments, start, end } => {
                let n = segments.len() as f64;
                // A closed path keeps its end bracket wherever it is (the same place as the start: a
                // full turn, see `TextKind::path_span`); an open one's end at its end is none.
                let end = (closed(segments) || *end < n - 0.02).then(|| length_fraction(segments, *end));
                (1.0, Affine::IDENTITY, TextKind::OnPath { path: path_of(segments, template, to_doc), start: length_fraction(segments, *start), end })
            }
        };
        let text: String = if threaded {
            String::new()
        } else {
            self.text.trim_end_matches('\r').chars().map(|c| if matches!(c, '\r' | '\x03') { '\n' } else { c }).collect()
        };
        if text.trim().is_empty() && !threaded {
            return None;
        }
        // The text document measures in the art's units too: the document's scale carries its
        // sizes and spacings onto the page (a Large Canvas copy is written at 1:10; #2031).
        let scale = scale * art_scale(to_doc);
        let leading = self.line_leading(scale);
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
            runs.push(TextRun::new(piece, char_style(st, scale, leading)));
        }
        if from < chars.len() {
            let last = self.runs.last().map(|r| &r.1)?;
            runs.push(TextRun::new(chars.get(from..).unwrap_or_default().iter().collect::<String>(), char_style(last, scale, leading)));
        }
        let first = self.runs.first().map(|r| char_style(&r.1, scale, leading))?;
        let mut t = TextObject::point(Point::ZERO, "", first);
        if !runs.is_empty() {
            t.runs = runs;
        }
        let paras: Vec<ParaStyle> = self.paragraph_styles(&text, scale);
        t.para = paras.first().cloned().unwrap_or_default();
        if paras.len() > 1 && paras.iter().any(|p| *p != t.para) {
            t.paras = paras;
        }
        t.kind = kind;
        t.xf = xf;
        Some(Node::new(id, NodeKind::Text(Box::new(t))))
    }

    /// Area type that the file lays lines out in, in a frame too short for its first line as it is
    /// laid out here: a font that isn't available shows in the fallback font, whose lines may be
    /// taller, and type that doesn't fit its frame isn't drawn. The frame is made taller, from its
    /// top, by what the first line needs (`vectorcraft_text::fit_first_line`). Whether it was.
    pub(super) fn fit_first_line(&self, t: &mut TextObject) -> bool {
        !self.lines.is_empty() && vectorcraft_text::fit_first_line(vectorcraft_text::FontDb::global(), t)
    }

    /// The leading the file's own line positions imply: the distance from one line to the next, less
    /// the space between paragraphs that is part of it (a paragraph's space after and the next one's
    /// space before; a layout's lines are where that space has put them, and the text object adds it
    /// again).
    fn line_leading(&self, scale: f64) -> Option<f64> {
        let para_at = |c: usize| {
            let mut end = 0usize;
            for (n, p) in &self.paras {
                end = end.saturating_add(*n);
                if c < end {
                    return Some(p);
                }
            }
            self.paras.last().map(|(_, p)| p)
        };
        let chars: Vec<char> = self.text.chars().collect();
        let breaks: Vec<usize> = chars.iter().enumerate().filter(|(_, c)| matches!(c, '\r' | '\x03')).map(|(i, _)| i).collect();
        // More lines than breaks: area type wraps, and which break ends which line is unknown.
        let wraps = self.lines.len() > breaks.len() + 1;
        self.lines.windows(2).enumerate().find_map(|(i, w)| {
            let pitch = w[1].1 - w[0].1;
            let between = match breaks.get(i) {
                Some(&at) if !wraps && chars.get(at) == Some(&'\r') => {
                    para_at(at).map_or(0.0, |p| p.space_after) + para_at(at + 1).map_or(0.0, |p| p.space_before)
                }
                _ => 0.0,
            };
            let l = (pitch - between) * scale;
            (l.is_finite() && l > 0.0).then_some(l)
        })
    }

    /// The style of each paragraph of `text` (paragraphs split at `\n`).
    fn paragraph_styles(&self, text: &str, scale: f64) -> Vec<ParaStyle> {
        let count = text.split('\n').count();
        let mut out = Vec::with_capacity(count);
        let mut runs = self.paras.iter();
        let mut current = runs.next();
        let mut left = current.map_or(usize::MAX, |r| r.0);
        for p in text.split('\n') {
            let para = current.map(|r| r.1.clone()).unwrap_or_default();
            out.push(para_style(&para, scale));
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
        ..ParaStyle::default()
    }
}

fn char_style(s: &Style, scale: f64, leading: Option<f64>) -> CharStyle {
    let (family, style) = s.font.as_deref().map_or_else(|| (CharStyle::default().font_family, "Regular".to_string()), crate::family_style);
    let family = vectorcraft_text::FontDb::global().find_family(&family).unwrap_or(family);
    CharStyle {
        font_family: family,
        font_style: style,
        size: s.size * scale,
        leading: s.leading.map(|l| l * scale).or(leading),
        tracking: s.tracking,
        baseline_shift: s.baseline_shift * scale,
        h_scale: s.h_scale * 100.0,
        v_scale: s.v_scale * 100.0,
        fill: s.fill.map_or(Paint::None, Paint::solid),
        stroke: s.stroke.map_or(Paint::None, Paint::solid),
        stroke_width: if s.stroke.is_some() { s.stroke_width * scale } else { 0.0 },
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
        let texts =
            Texts { stories: vec![], fonts: vec![], defaults: Val::Dict(BTreeMap::new()), para_defaults: read(b"/33 1").unwrap(), frames: vec![] };
        let justify = |own: &[u8]| para_style(&texts.para(Some(&read(own).unwrap())), 1.0).justify;
        // Right to left from the document's defaults, or the paragraph's own.
        assert_eq!(justify(b"/0 0"), Justify::Right);
        assert_eq!(justify(b"/0 1 /33 1"), Justify::Left);
        assert_eq!(justify(b"/0 2"), Justify::Center);
        // Left to right keeps them.
        assert_eq!(justify(b"/0 0 /33 0"), Justify::Left);
        assert_eq!(justify(b"/0 1 /33 0"), Justify::Right);
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
        let nums = |v: &[f64]| v.iter().map(|n| format!("{n}")).collect::<Vec<_>>().join(" ");
        let all: Vec<f64> = segments.iter().flatten().copied().collect();
        let doc = format!("/0 << /1 << /0 [ {} ] >> /2 << /0 2 /6 [ {} ] >> >>", nums(&all), nums(&range));
        let texts = Texts {
            stories: vec![],
            fonts: vec![],
            defaults: Val::Dict(BTreeMap::new()),
            para_defaults: Val::Dict(BTreeMap::new()),
            frames: vec![read(doc.as_bytes()).unwrap()],
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
                    stroke_width: 1.0,
                },
            )],
        }
    }

    /// The leading of the text object a story comes in as.
    fn leading_of(story: &Story) -> Option<f64> {
        let node = story.node(NodeId(1), 0, (0.0, 0.0), Affine::IDENTITY)?;
        match &node.kind {
            NodeKind::Text(t) => t.runs.first().and_then(|r| r.style.leading),
            _ => None,
        }
    }

    #[test]
    fn leading_leaves_out_the_space_between_paragraphs_that_the_lines_already_have() {
        // Lines 10.6 apart that are three paragraphs with 5 less after each: auto leading (15.6) less 5.
        let para = Para { space_after: -5.0, ..Para::default() };
        let s = story("one\rtwo\rthree\r", &[0.0, 10.6, 21.2], &para);
        assert!((s.line_leading(1.0).unwrap() - 15.6).abs() < 1e-9);
        assert!((leading_of(&s).unwrap() - 15.6).abs() < 1e-9);
        // Space after and space before both count; the scale of the frame carries onto the leading.
        let para = Para { space_after: 4.0, space_before: 2.0, ..Para::default() };
        let s = story("one\rtwo\r", &[0.0, 20.0], &para);
        assert!((s.line_leading(1.0).unwrap() - 14.0).abs() < 1e-9);
        assert!((s.line_leading(0.5).unwrap() - 7.0).abs() < 1e-9);
    }

    /// #2031: a Large Canvas editing copy measures its text document in its art's units, a tenth
    /// of the page's: the story's size and leading reach the page through `to_doc`'s scale.
    #[test]
    fn a_story_in_a_scaled_art_space_keeps_its_size_and_leading() {
        let s = story("one\rtwo\r", &[0.0, 20.0], &Para::default());
        // A copy written at 1:10 (see `ai::read::ensure_space`): its art's units reach the
        // document, 100 points to the page, through `to_doc`'s scale.
        let to_doc = Affine::scale(10.0) * Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 100.0]);
        let t = match s.node(NodeId(1), 0, (0.0, 0.0), to_doc).unwrap().kind {
            NodeKind::Text(t) => t,
            _ => panic!("not text"),
        };
        assert_eq!(t.runs[0].style.size, 130.0, "13 pt at the page's scale");
        assert_eq!(t.runs[0].style.leading, Some(200.0), "a 20-unit line pitch at the page's scale");
    }

    #[test]
    fn leading_between_lines_of_a_paragraph_is_their_distance() {
        // A line break inside a paragraph has no paragraph spacing between its lines.
        let para = Para { space_after: -5.0, space_before: 3.0, ..Para::default() };
        let s = story("one\x03two\x03three\r", &[0.0, 15.6, 31.2], &para);
        assert!((s.line_leading(1.0).unwrap() - 15.6).abs() < 1e-9);
        // Paragraphs without spacing: the distance as it was.
        let s = story("one\rtwo\r", &[0.0, 15.6], &Para::default());
        assert!((s.line_leading(1.0).unwrap() - 15.6).abs() < 1e-9);
        // Area type whose paragraph wraps: lines and breaks don't pair up, so nothing is taken off.
        let para = Para { space_after: -5.0, ..Para::default() };
        let s = story("one two three\rfour\r", &[0.0, 15.6, 31.2, 41.8], &para);
        assert!((s.line_leading(1.0).unwrap() - 15.6).abs() < 1e-9);
        // One line has no distance to read.
        assert_eq!(story("one\r", &[0.0], &Para::default()).line_leading(1.0), None);
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
}
