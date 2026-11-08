//! Compiling and matching regular expressions (ECMA-262 5.1 section 15.10.2, with the lenient
//! patterns of Annex B that [`crate::regexp::check`] accepts).
//!
//! A pattern compiles to a list of instructions for a backtracking matcher. The compiler reads the
//! pattern with a loop and a stack of open groups, and the matcher keeps its choice points and the
//! values it must restore on its own stack, so neither recurses however deeply the groups nest.
//! Quantifiers count their iterations in registers instead of repeating their atom's code. Every
//! instruction run, every capture slot cleared, every choice point taken back and every 16 units a
//! backreference compares cost fuel, and
//! the compiler's work, the matcher's state and its stack are held against the heap limit, so a
//! pattern that backtracks without end stops at a limit.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::LimitKind;
use crate::interp::{Abrupt, HeldBytes, Interp, JsResult};

/// A capture position that is not set.
pub(crate) const NONE: usize = usize::MAX;

/// Steps the matcher counts before it takes them from the fuel.
const FUEL_BATCH: u64 = 1024;

#[derive(Clone, Copy, Debug)]
enum Inst {
    /// One code unit, already canonicalized when the pattern ignores case.
    Char(u16),
    /// `.`: any unit but a line terminator.
    Any,
    Set(u32),
    LineStart,
    LineEnd,
    /// `\b` (true) or `\B` (false).
    WordBoundary(bool),
    /// Goes on at the first offset, and at the second when that fails.
    Split(i32, i32),
    Jump(i32),
    GroupStart(u32),
    GroupEnd(u32),
    /// Sets a quantifier's iteration count to zero.
    RepInit(u32),
    /// A quantifier's head: its atom follows, then [`Inst::RepEnd`]. `caps` is the range of groups
    /// inside the atom, cleared before each iteration; `exit` leads past the atom.
    Rep {
        reg: u32,
        min: u64,
        max: u64,
        greedy: bool,
        caps: (u32, u32),
        exit: i32,
    },
    RepEnd {
        reg: u32,
        head: i32,
    },
    /// A lookahead: its pattern follows, then [`Inst::LookEnd`]; `end` leads past it.
    Look {
        negative: bool,
        end: i32,
    },
    LookEnd,
    BackRef(u32),
    Match,
}

/// A character class: sorted, merged ranges of code units.
#[derive(Clone, Debug, Default)]
struct CharSet {
    ranges: Vec<(u16, u16)>,
    negated: bool,
}

impl CharSet {
    fn contains_raw(&self, u: u16) -> bool {
        let i = self.ranges.partition_point(|&(_, hi)| hi < u);
        self.ranges.get(i).is_some_and(|&(lo, _)| lo <= u)
    }
}

/// The flags of a regular expression (section 15.10.4.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Flags {
    pub(crate) global: bool,
    pub(crate) ignore_case: bool,
    pub(crate) multiline: bool,
}

/// Reads flags: each of `g`, `i` and `m` at most once.
pub(crate) fn parse_flags(units: &[u16]) -> Result<Flags, String> {
    let mut f = Flags::default();
    for &u in units {
        let slot = match u {
            0x67 => &mut f.global,
            0x69 => &mut f.ignore_case,
            0x6D => &mut f.multiline,
            _ => return Err(format!("invalid regular expression flag '{}'", String::from_utf16_lossy(&[u]))),
        };
        if *slot {
            return Err(format!("regular expression flag '{}' given twice", String::from_utf16_lossy(&[u])));
        }
        *slot = true;
    }
    Ok(f)
}

/// A compiled regular expression.
pub(crate) struct Regex {
    insts: Vec<Inst>,
    sets: Vec<CharSet>,
    /// Capturing groups, not counting the whole match.
    pub(crate) groups: usize,
    loops: usize,
    pub(crate) ignore_case: bool,
    pub(crate) multiline: bool,
}

/// Why a pattern did not compile.
pub(crate) enum CompileFail {
    Syntax(String),
    Abrupt(Abrupt),
}

impl From<String> for CompileFail {
    fn from(s: String) -> Self {
        CompileFail::Syntax(s)
    }
}

impl From<&str> for CompileFail {
    fn from(s: &str) -> Self {
        CompileFail::Syntax(s.into())
    }
}

/// What compiling takes as it goes: fuel for the work done, or heap bytes for what it holds.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Cost {
    Fuel(u64),
    Heap(usize),
}

impl Regex {
    /// The bytes the compiled form holds, counting the capacity of each list.
    pub(crate) fn heap_bytes(&self) -> usize {
        let sets = self.sets.iter().map(set_bytes).fold(0usize, usize::saturating_add);
        size_of::<Regex>()
            .saturating_add(size_of::<Inst>().saturating_mul(self.insts.capacity()))
            .saturating_add(size_of::<CharSet>().saturating_mul(self.sets.capacity()))
            .saturating_add(sets)
    }

    fn canon(&self, u: u16) -> u16 {
        if self.ignore_case { canonicalize(u) } else { u }
    }

    fn set_matches(&self, set: &CharSet, u: u16) -> bool {
        let found = if self.ignore_case {
            let k = canonicalize(u);
            match case_groups().get(&k) {
                Some(group) => group.iter().any(|&c| set.contains_raw(c)),
                None => set.contains_raw(u),
            }
        } else {
            set.contains_raw(u)
        };
        found != set.negated
    }
}

// ---- characters ----

fn is_line_terminator(u: u16) -> bool {
    crate::chars::is_line_terminator(u32::from(u))
}

fn is_word_char(u: u16) -> bool {
    matches!(u, 0x30..=0x39 | 0x41..=0x5A | 0x5F | 0x61..=0x7A)
}

/// Canonicalize (section 15.10.2.8): the unit's upper case when that is one unit, unless it
/// would turn a unit from 128 up into one below 128.
fn canonicalize(u: u16) -> u16 {
    static TABLE: OnceLock<Vec<u16>> = OnceLock::new();
    let table = TABLE.get_or_init(|| (0..=u16::MAX).map(upper_unit).collect());
    table.get(usize::from(u)).copied().unwrap_or(u)
}

fn upper_unit(u: u16) -> u16 {
    let Some(c) = char::from_u32(u32::from(u)) else { return u };
    let mut upper = c.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(up), None) => match u16::try_from(u32::from(up)) {
            Ok(cu) if !(u >= 128 && cu < 128) => cu,
            _ => u,
        },
        _ => u,
    }
}

/// For each canonical unit that more than one unit canonicalizes to, those units.
fn case_groups() -> &'static HashMap<u16, Vec<u16>> {
    static GROUPS: OnceLock<HashMap<u16, Vec<u16>>> = OnceLock::new();
    GROUPS.get_or_init(|| {
        let mut groups: HashMap<u16, Vec<u16>> = HashMap::new();
        for u in 0..=u16::MAX {
            let k = canonicalize(u);
            if k != u {
                groups.entry(k).or_default().push(u);
            }
        }
        for (k, members) in &mut groups {
            if canonicalize(*k) == *k {
                members.push(*k);
            }
        }
        groups
    })
}

fn digit_ranges() -> Vec<(u16, u16)> {
    vec![(0x30, 0x39)]
}

fn word_ranges() -> Vec<(u16, u16)> {
    vec![(0x30, 0x39), (0x41, 0x5A), (0x5F, 0x5F), (0x61, 0x7A)]
}

/// White space and line terminators (`\s`).
fn space_ranges() -> Vec<(u16, u16)> {
    let mut out: Vec<(u16, u16)> = Vec::new();
    for u in 0..=u16::MAX {
        if crate::value::is_str_space(u) {
            match out.last_mut() {
                Some((_, hi)) if u32::from(*hi) + 1 == u32::from(u) => *hi = u,
                _ => out.push((u, u)),
            }
        }
    }
    out
}

fn complement(ranges: &[(u16, u16)]) -> Vec<(u16, u16)> {
    let mut out = Vec::new();
    let mut next: u32 = 0;
    for &(lo, hi) in ranges {
        if u32::from(lo) > next {
            out.push((next as u16, lo - 1));
        }
        next = u32::from(hi) + 1;
    }
    if next <= 0xFFFF {
        out.push((next as u16, 0xFFFF));
    }
    out.shrink_to_fit();
    out
}

/// Sorts and merges ranges.
fn normalize(mut ranges: Vec<(u16, u16)>) -> Vec<(u16, u16)> {
    ranges.sort_unstable();
    let mut out: Vec<(u16, u16)> = Vec::with_capacity(ranges.len());
    for (lo, hi) in ranges {
        match out.last_mut() {
            Some((_, last)) if u32::from(lo) <= u32::from(*last) + 1 => *last = (*last).max(hi),
            _ => out.push((lo, hi)),
        }
    }
    out.shrink_to_fit();
    out
}

/// The ranges of a class escape letter (`d`, `D`, `s`, `S`, `w` or `W`).
fn escape_ranges(letter: u16) -> Option<Vec<(u16, u16)>> {
    static SPACE: OnceLock<Vec<(u16, u16)>> = OnceLock::new();
    let space = || SPACE.get_or_init(space_ranges).clone();
    Some(match letter {
        0x64 => digit_ranges(),
        0x44 => complement(&digit_ranges()),
        0x73 => space(),
        0x53 => complement(&space()),
        0x77 => word_ranges(),
        0x57 => complement(&word_ranges()),
        _ => return None,
    })
}

// ---- compiling ----

#[derive(Clone, Copy)]
enum Kind {
    Top,
    Capture(u32),
    Group,
    Look(bool),
}

/// A piece of code being compiled: one instruction, or a finished fragment placed whole, so that
/// closing a group or quantifying it moves one item however much code it holds.
#[derive(Clone, Copy)]
enum Item {
    Inst(Inst),
    Frag(usize),
}

/// The finished fragments, with the number of instructions each one flattens to.
#[derive(Default)]
struct Frags {
    items: Vec<Vec<Item>>,
    lens: Vec<usize>,
}

impl Frags {
    fn len_of(&self, items: &[Item]) -> usize {
        items
            .iter()
            .map(|i| match i {
                Item::Inst(_) => 1,
                Item::Frag(f) => self.lens.get(*f).copied().unwrap_or(0),
            })
            .fold(0usize, usize::saturating_add)
    }

    fn add(&mut self, items: Vec<Item>) -> Item {
        let len = self.len_of(&items);
        self.items.push(items);
        self.lens.push(len);
        Item::Frag(self.items.len() - 1)
    }

    /// The instructions of `top` with every fragment in place, written with a loop, with room for
    /// one more. Its stack holds at most one entry for each fragment and two more, so it never
    /// grows past [`flatten_bytes`].
    fn flatten(&self, top: &[Item]) -> Vec<Inst> {
        let mut out = Vec::with_capacity(self.len_of(top).saturating_add(1));
        let mut stack: Vec<(&[Item], usize)> = Vec::with_capacity(self.items.len().saturating_add(2));
        stack.push((top, 0));
        while let Some((items, i)) = stack.pop() {
            let Some(item) = items.get(i) else { continue };
            stack.push((items, i + 1));
            match item {
                Item::Inst(inst) => out.push(*inst),
                Item::Frag(f) => {
                    if let Some(child) = self.items.get(*f) {
                        stack.push((child, 0));
                    }
                }
            }
        }
        out
    }
}

struct Ctx {
    kind: Kind,
    alts: Vec<Vec<Item>>,
    cur: Vec<Item>,
    /// The last term, when it may take a quantifier: where it starts in `cur` and how many groups
    /// were opened before it.
    atom: Option<(usize, u32)>,
    /// Where the group starts in its parent's `cur` and how many groups were opened before it.
    opened_at: (usize, u32),
}

impl Ctx {
    fn new(kind: Kind, opened_at: (usize, u32)) -> Self {
        Self { kind, alts: Vec::new(), cur: Vec::new(), atom: None, opened_at }
    }
}

/// Adds `inst` to `ctx`.
fn emit(ctx: &mut Ctx, inst: Inst) {
    ctx.cur.push(Item::Inst(inst));
}

/// The bytes of one item of code being compiled.
pub(crate) const ITEM_BYTES: usize = size_of::<Item>();

/// The bytes of one instruction of compiled code.
pub(crate) const INST_BYTES: usize = size_of::<Inst>();

/// Charges `bytes` that compiling holds.
fn hold(bytes: usize, charge: &mut dyn FnMut(Cost) -> JsResult<()>) -> Result<(), CompileFail> {
    if bytes == 0 {
        return Ok(());
    }
    charge(Cost::Heap(bytes)).map_err(CompileFail::Abrupt)
}

/// Makes room in `list` for `extra` more entries. When it must grow, its capacity at least
/// doubles, and the bytes of the added capacity are charged before it is reserved.
fn room<T>(list: &mut Vec<T>, extra: usize, charge: &mut dyn FnMut(Cost) -> JsResult<()>) -> Result<(), CompileFail> {
    let need = list.len().saturating_add(extra);
    let cap = list.capacity();
    if need <= cap {
        return Ok(());
    }
    let new_cap = need.max(cap.saturating_mul(2));
    hold(size_of::<T>().saturating_mul(new_cap - cap), charge)?;
    list.reserve_exact(new_cap - list.len());
    Ok(())
}

/// The bytes `join` holds for `alts` alternatives besides the fragments it adds: the code that
/// joins them and their lengths.
fn join_bytes(alts: usize) -> usize {
    if alts <= 1 {
        return 0;
    }
    alts.saturating_mul(2 * ITEM_BYTES + size_of::<usize>())
}

/// The bytes flattening holds: `instructions` and one more, and its stack for `fragments`.
fn flatten_bytes(instructions: usize, fragments: usize) -> usize {
    INST_BYTES
        .saturating_mul(instructions.saturating_add(1))
        .saturating_add(size_of::<(&[Item], usize)>().saturating_mul(fragments.saturating_add(2)))
}

fn offset(n: usize) -> Result<i32, CompileFail> {
    i32::try_from(n).map_err(|_| CompileFail::Syntax("regular expression too large".into()))
}

/// The alternatives of a disjunction as one piece of code.
fn join(mut alts: Vec<Vec<Item>>, frags: &mut Frags) -> Result<Vec<Item>, CompileFail> {
    if alts.len() == 1 {
        return Ok(alts.pop().unwrap_or_default());
    }
    let lens: Vec<usize> = alts.iter().map(|a| frags.len_of(a)).collect();
    let total = lens.iter().sum::<usize>() + 2 * alts.len().saturating_sub(1);
    let last = alts.len().saturating_sub(1);
    let mut out = Vec::with_capacity(2 * alts.len());
    let mut at = 0usize;
    for (i, (a, len)) in alts.into_iter().zip(lens).enumerate() {
        if i == last {
            out.push(frags.add(a));
        } else {
            out.push(Item::Inst(Inst::Split(1, offset(len + 2)?)));
            out.push(frags.add(a));
            at += len + 1;
            out.push(Item::Inst(Inst::Jump(offset(total - at)?)));
            at += 1;
        }
    }
    Ok(out)
}

/// The number of capturing groups in `pattern`.
fn count_groups(pattern: &[u16]) -> usize {
    let mut i = 0;
    let mut n = 0;
    let mut in_class = false;
    while let Some(&u) = pattern.get(i) {
        match u {
            0x5C => i += 1,
            0x5B => in_class = true,
            0x5D => in_class = false,
            0x28 if !in_class && pattern.get(i + 1) != Some(&0x3F) => n += 1,
            _ => {}
        }
        i += 1;
    }
    n
}

fn hex_digits(pattern: &[u16], at: usize, n: usize) -> Option<u16> {
    let mut v: u32 = 0;
    for k in 0..n {
        let d = crate::chars::hex_value(u32::from(*pattern.get(at + k)?))?;
        v = v * 16 + d;
    }
    u16::try_from(v).ok()
}

/// A legacy octal escape (Annex B) whose first digit is at `at`: its value and the position after
/// it.
fn octal(pattern: &[u16], at: usize) -> (u16, usize) {
    let digit = |i: usize| pattern.get(i).filter(|&&u| (0x30..=0x37).contains(&u)).map(|&u| u - 0x30);
    let Some(first) = digit(at) else { return (0, at) };
    let max = if first <= 3 { 3 } else { 2 };
    let mut value = first;
    let mut i = at + 1;
    while i < at + max
        && let Some(d) = digit(i)
    {
        value = value * 8 + d;
        i += 1;
    }
    (value, i)
}

/// A `{n}`, `{n,}` or `{n,m}` quantifier whose `{` is at `i`: its bounds and the position after it.
fn braces(pattern: &[u16], i: usize) -> Option<(u64, u64, usize)> {
    let digits = |mut j: usize| -> Option<(u64, usize)> {
        let start = j;
        let mut v: u64 = 0;
        while let Some(d) = pattern.get(j).filter(|&&u| (0x30..=0x39).contains(&u)) {
            v = v.saturating_mul(10).saturating_add(u64::from(d - 0x30));
            j += 1;
        }
        (j > start).then_some((v, j))
    };
    let (min, mut j) = digits(i + 1)?;
    let max = if pattern.get(j) == Some(&0x2C) {
        j += 1;
        match digits(j) {
            Some((max, end)) => {
                j = end;
                max
            }
            None => u64::MAX,
        }
    } else {
        min
    };
    (pattern.get(j) == Some(&0x7D)).then_some((min, max, j + 1))
}

/// A class atom: one unit, or the ranges of a class escape.
enum ClassAtom {
    Unit(u16),
    Ranges(Vec<(u16, u16)>),
}

/// The class atom at `i` and the position after it.
fn class_atom(pattern: &[u16], i: usize) -> Result<(ClassAtom, usize), CompileFail> {
    let unit = *pattern.get(i).ok_or("unterminated character class")?;
    if unit != 0x5C {
        return Ok((ClassAtom::Unit(unit), i + 1));
    }
    let next = *pattern.get(i + 1).ok_or("\\ at end of pattern")?;
    if let Some(r) = escape_ranges(next) {
        return Ok((ClassAtom::Ranges(r), i + 2));
    }
    let unit = |u: u16, end: usize| Ok((ClassAtom::Unit(u), end));
    match next {
        0x62 => unit(0x08, i + 2),
        0x66 => unit(0x0C, i + 2),
        0x6E => unit(0x0A, i + 2),
        0x72 => unit(0x0D, i + 2),
        0x74 => unit(0x09, i + 2),
        0x76 => unit(0x0B, i + 2),
        0x78 => hex_digits(pattern, i + 2, 2).map_or(unit(next, i + 2), |v| unit(v, i + 4)),
        0x75 => hex_digits(pattern, i + 2, 4).map_or(unit(next, i + 2), |v| unit(v, i + 6)),
        0x63 => match pattern.get(i + 2) {
            Some(&l) if matches!(l, 0x30..=0x39 | 0x41..=0x5A | 0x61..=0x7A | 0x5F) => unit(l % 32, i + 3),
            _ => unit(0x5C, i + 1),
        },
        0x30..=0x37 => {
            let (v, end) = octal(pattern, i + 1);
            unit(v, end)
        }
        _ => unit(next, i + 2),
    }
}

/// Counts `n` units of work in `work`, taking them from the fuel once a batch is full.
fn add_work(work: &mut u64, n: u64, charge: &mut dyn FnMut(Cost) -> JsResult<()>) -> Result<(), CompileFail> {
    *work = work.saturating_add(n);
    if *work >= FUEL_BATCH {
        charge(Cost::Fuel(std::mem::take(work))).map_err(CompileFail::Abrupt)?;
    }
    Ok(())
}

/// The ranges of a class being read, with the bytes charged for them so far.
struct ClassRanges {
    ranges: Vec<(u16, u16)>,
    held: usize,
}

impl ClassRanges {
    /// Adds the ranges of an atom.
    fn add_atom(&mut self, atom: &ClassAtom, work: &mut u64, charge: &mut dyn FnMut(Cost) -> JsResult<()>) -> Result<(), CompileFail> {
        match atom {
            ClassAtom::Unit(u) => self.add(&[(*u, *u)], work, charge),
            ClassAtom::Ranges(r) => self.add(r, work, charge),
        }
    }

    /// Adds `part`. Each range counts as work, and when the list must grow, the bytes of its new
    /// capacity and of the copy `normalize` makes of it are charged first.
    fn add(&mut self, part: &[(u16, u16)], work: &mut u64, charge: &mut dyn FnMut(Cost) -> JsResult<()>) -> Result<(), CompileFail> {
        let extra = part.len();
        add_work(work, extra as u64, charge)?;
        let len = self.ranges.len().saturating_add(extra);
        if len > self.ranges.capacity() {
            let cap = len.max(self.ranges.capacity().saturating_mul(2));
            let need = cap.saturating_mul(2 * size_of::<(u16, u16)>());
            if need > self.held {
                charge(Cost::Heap(need - self.held)).map_err(CompileFail::Abrupt)?;
                self.held = need;
            }
            self.ranges.reserve_exact(cap - self.ranges.len());
        }
        self.ranges.extend_from_slice(part);
        Ok(())
    }
}

/// The character class whose `[` is at `i`, and the position after its `]`. Each atom read and
/// each range added counts as work in `work`, and the class's ranges are charged as they grow.
fn class(pattern: &[u16], mut i: usize, work: &mut u64, charge: &mut dyn FnMut(Cost) -> JsResult<()>) -> Result<(CharSet, usize), CompileFail> {
    i += 1;
    let negated = pattern.get(i) == Some(&0x5E);
    if negated {
        i += 1;
    }
    let mut list = ClassRanges { ranges: Vec::new(), held: 0 };
    loop {
        add_work(work, 1, charge)?;
        let u = *pattern.get(i).ok_or("unterminated character class")?;
        if u == 0x5D {
            return Ok((CharSet { ranges: normalize(list.ranges), negated }, i + 1));
        }
        let (atom, next) = class_atom(pattern, i)?;
        i = next;
        let dash = pattern.get(i) == Some(&0x2D) && pattern.get(i + 1).is_some_and(|&u| u != 0x5D);
        if dash {
            let (high, after) = class_atom(pattern, i + 1)?;
            match (atom, high) {
                (ClassAtom::Unit(lo), ClassAtom::Unit(hi)) => {
                    if lo > hi {
                        return Err("range out of order in character class".into());
                    }
                    list.add(&[(lo, hi)], work, charge)?;
                }
                // Annex B: a range with a class escape at one end is its atoms and a `-`.
                (lo, hi) => {
                    for a in [lo, ClassAtom::Unit(0x2D), hi] {
                        list.add_atom(&a, work, charge)?;
                    }
                }
            }
            i = after;
        } else {
            list.add_atom(&atom, work, charge)?;
        }
    }
}

/// At least the bytes of a class escape's ranges (`\d`, `\S`, ...).
const ESCAPE_RANGES_BYTES: usize = 16 * size_of::<(u16, u16)>();

/// The bytes a character class's ranges hold.
fn set_bytes(set: &CharSet) -> usize {
    size_of::<(u16, u16)>().saturating_mul(set.ranges.capacity())
}

/// Compiles `pattern`. Groups may nest `max_depth` deep. `charge` takes fuel for the work done,
/// counting each unit of the scan for capturing groups and of the compile itself, and heap bytes
/// for what compiling holds before it is made: the capacity each list reserves as it grows, the
/// ranges of each class, and the code that joins alternatives and the flattened code. An error
/// from `charge` stops compiling.
pub(crate) fn compile(pattern: &[u16], flags: Flags, max_depth: usize, charge: &mut dyn FnMut(Cost) -> JsResult<()>) -> Result<Regex, CompileFail> {
    let mut work: u64 = 0;
    add_work(&mut work, pattern.len() as u64, charge)?;
    let total_groups = count_groups(pattern);
    hold(size_of::<Ctx>(), charge)?;
    let mut sets: Vec<CharSet> = Vec::new();
    let mut stack: Vec<Ctx> = vec![Ctx::new(Kind::Top, (0, 0))];
    let mut frags = Frags::default();
    let mut groups: u32 = 0;
    let mut loops: u32 = 0;
    let mut i = 0;
    let canon = |u: u16| if flags.ignore_case { canonicalize(u) } else { u };
    while let Some(&u) = pattern.get(i) {
        add_work(&mut work, 1, charge)?;
        // One step adds at most one open group, one class, one alternative and four items to the
        // current alternative, so those lists have room for them before the step.
        room(&mut stack, 1, charge)?;
        room(&mut sets, 1, charge)?;
        let ctx = stack.last_mut().ok_or("unmatched ')'")?;
        room(&mut ctx.cur, 4, charge)?;
        room(&mut ctx.alts, 1, charge)?;
        let start = ctx.cur.len();
        let before = groups;
        match u {
            0x5E | 0x24 => {
                emit(ctx, if u == 0x5E { Inst::LineStart } else { Inst::LineEnd });
                ctx.atom = None;
                i += 1;
            }
            0x7C => {
                let cur = std::mem::take(&mut ctx.cur);
                ctx.alts.push(cur);
                ctx.atom = None;
                i += 1;
            }
            0x2E => {
                emit(ctx, Inst::Any);
                ctx.atom = Some((start, before));
                i += 1;
            }
            0x5B => {
                let (set, next) = class(pattern, i, &mut work, charge)?;
                emit(ctx, Inst::Set(u32::try_from(sets.len()).map_err(|_| "too many classes")?));
                sets.push(set);
                ctx.atom = Some((start, before));
                i = next;
            }
            0x28 => {
                let kind = match (pattern.get(i + 1), pattern.get(i + 2)) {
                    (Some(0x3F), Some(0x3A)) => Kind::Group,
                    (Some(0x3F), Some(0x3D)) => Kind::Look(false),
                    (Some(0x3F), Some(0x21)) => Kind::Look(true),
                    (Some(0x3F), _) => return Err("invalid group".into()),
                    _ => {
                        groups += 1;
                        Kind::Capture(groups)
                    }
                };
                i += if matches!(kind, Kind::Capture(_)) { 1 } else { 3 };
                if stack.len() > max_depth {
                    return Err(crate::error::TOO_DEEP.into());
                }
                stack.push(Ctx::new(kind, (start, before)));
            }
            0x29 => {
                if stack.len() <= 1 {
                    return Err("unmatched ')'".into());
                }
                let Some(mut done) = stack.pop() else { return Err("unmatched ')'".into()) };
                let cur = std::mem::take(&mut done.cur);
                done.alts.push(cur);
                // Joining adds a fragment for each alternative when there are several, and the
                // group's body is one more.
                let alts = done.alts.len();
                hold(join_bytes(alts), charge)?;
                room(&mut frags.items, alts + 1, charge)?;
                room(&mut frags.lens, alts + 1, charge)?;
                let body = join(std::mem::take(&mut done.alts), &mut frags)?;
                work += body.len() as u64;
                let body_len = frags.len_of(&body);
                let body = frags.add(body);
                let (start, before) = done.opened_at;
                let parent = stack.last_mut().ok_or("unmatched ')'")?;
                room(&mut parent.cur, 3, charge)?;
                match done.kind {
                    Kind::Capture(n) => {
                        emit(parent, Inst::GroupStart(n));
                        parent.cur.push(body);
                        emit(parent, Inst::GroupEnd(n));
                    }
                    Kind::Group | Kind::Top => parent.cur.push(body),
                    Kind::Look(negative) => {
                        let end = offset(body_len + 2)?;
                        emit(parent, Inst::Look { negative, end });
                        parent.cur.push(body);
                        emit(parent, Inst::LookEnd);
                    }
                }
                parent.atom = Some((start, before));
                i += 1;
            }
            0x2A | 0x2B | 0x3F | 0x7B => {
                let (min, max, next) = match u {
                    0x2A => (0, u64::MAX, i + 1),
                    0x2B => (1, u64::MAX, i + 1),
                    0x3F => (0, 1, i + 1),
                    _ => match braces(pattern, i) {
                        Some(b) => b,
                        None => {
                            // Annex B: a `{` that starts no quantifier is itself.
                            emit(ctx, Inst::Char(canon(u)));
                            ctx.atom = Some((start, before));
                            i += 1;
                            continue;
                        }
                    },
                };
                if max < min {
                    return Err("numbers out of order in {} quantifier".into());
                }
                let Some((atom_start, atom_groups)) = ctx.atom.take() else { return Err("nothing to repeat".into()) };
                let greedy = pattern.get(next) != Some(&0x3F);
                i = if greedy { next } else { next + 1 };
                if min == 1 && max == 1 {
                    continue;
                }
                hold(ITEM_BYTES.saturating_mul(ctx.cur.len().saturating_sub(atom_start)), charge)?;
                room(&mut frags.items, 1, charge)?;
                room(&mut frags.lens, 1, charge)?;
                let body: Vec<Item> = ctx.cur.drain(atom_start..).collect();
                work += body.len() as u64;
                let reg = loops;
                loops = loops.checked_add(1).ok_or("too many quantifiers")?;
                let n = frags.len_of(&body);
                let body = frags.add(body);
                emit(ctx, Inst::RepInit(reg));
                emit(ctx, Inst::Rep { reg, min, max, greedy, caps: (atom_groups + 1, groups + 1), exit: offset(n + 2)? });
                ctx.cur.push(body);
                emit(ctx, Inst::RepEnd { reg, head: -offset(n + 1)? });
            }
            0x5C => {
                let next = *pattern.get(i + 1).ok_or("\\ at end of pattern")?;
                let mut atom = true;
                match next {
                    0x62 | 0x42 => {
                        emit(ctx, Inst::WordBoundary(next == 0x62));
                        atom = false;
                        i += 2;
                    }
                    0x31..=0x39 => {
                        let mut j = i + 1;
                        let mut n: u64 = 0;
                        while let Some(d) = pattern.get(j).filter(|&&u| (0x30..=0x39).contains(&u)) {
                            n = n.saturating_mul(10).saturating_add(u64::from(d - 0x30));
                            j += 1;
                        }
                        if n <= total_groups as u64 {
                            emit(ctx, Inst::BackRef(u32::try_from(n).map_err(|_| "too many groups")?));
                            i = j;
                        } else if next <= 0x37 {
                            let (v, end) = octal(pattern, i + 1);
                            emit(ctx, Inst::Char(canon(v)));
                            i = end;
                        } else {
                            emit(ctx, Inst::Char(canon(next)));
                            i += 2;
                        }
                    }
                    0x30 => {
                        if pattern.get(i + 2).is_some_and(|&d| (0x30..=0x37).contains(&d)) {
                            let (v, end) = octal(pattern, i + 1);
                            emit(ctx, Inst::Char(canon(v)));
                            i = end;
                        } else {
                            emit(ctx, Inst::Char(0));
                            i += 2;
                        }
                    }
                    _ => {
                        if matches!(next, 0x64 | 0x44 | 0x73 | 0x53 | 0x77 | 0x57) {
                            hold(ESCAPE_RANGES_BYTES, charge)?;
                        }
                        if let Some(r) = escape_ranges(next) {
                            emit(ctx, Inst::Set(u32::try_from(sets.len()).map_err(|_| "too many classes")?));
                            sets.push(CharSet { ranges: r, negated: false });
                            i += 2;
                        } else {
                            let (atom_unit, end) = match class_atom(pattern, i)? {
                                (ClassAtom::Unit(v), end) => (v, end),
                                (ClassAtom::Ranges(_), end) => (next, end),
                            };
                            emit(ctx, Inst::Char(canon(atom_unit)));
                            i = end;
                        }
                    }
                }
                ctx.atom = if atom { Some((start, before)) } else { None };
            }
            _ => {
                emit(ctx, Inst::Char(canon(u)));
                ctx.atom = Some((start, before));
                i += 1;
            }
        }
    }
    if stack.len() != 1 {
        return Err("unterminated group".into());
    }
    let Some(mut top) = stack.pop() else { return Err("unterminated group".into()) };
    let cur = std::mem::take(&mut top.cur);
    top.alts.push(cur);
    // Joining adds a fragment for each alternative when there are several, and flattening writes
    // every instruction.
    let alts = top.alts.len();
    hold(join_bytes(alts), charge)?;
    room(&mut frags.items, alts, charge)?;
    room(&mut frags.lens, alts, charge)?;
    let top = join(top.alts, &mut frags)?;
    hold(flatten_bytes(frags.len_of(&top), frags.items.len()), charge)?;
    let mut insts = frags.flatten(&top);
    insts.push(Inst::Match);
    sets.shrink_to_fit();
    charge(Cost::Fuel(work + insts.len() as u64 / 16)).map_err(CompileFail::Abrupt)?;
    Ok(Regex { insts, sets, groups: groups as usize, loops: loops as usize, ignore_case: flags.ignore_case, multiline: flags.multiline })
}

// ---- matching ----

/// An entry on the matcher's stack: a choice point to go back to, or a value to restore when
/// going back past it.
#[derive(Clone, Copy, Debug)]
enum Bt {
    Retry {
        pc: usize,
        pos: usize,
    },
    /// Runs another iteration of the quantifier at `pc`.
    EnterRep {
        pc: usize,
        pos: usize,
    },
    Cap(usize, usize),
    Open(usize, usize),
    Count(usize, u64),
    Mark(usize, usize),
    Look {
        pos: usize,
        negative: bool,
        resume: usize,
    },
}

impl Bt {
    fn is_undo(&self) -> bool {
        matches!(self, Bt::Cap(..) | Bt::Open(..) | Bt::Count(..) | Bt::Mark(..))
    }
}

/// Takes `n` units of fuel, checking the interrupt flag.
pub(crate) fn use_fuel_n(interp: &mut Interp, n: u64) -> JsResult<()> {
    if n == 0 {
        return Ok(());
    }
    if interp.fuel < n {
        interp.fuel = 0;
        return Err(Abrupt::Limit(LimitKind::Fuel));
    }
    interp.fuel -= n - 1;
    interp.use_fuel()
}

/// The stack entries a matcher's stack first grows by.
const STACK_CHUNK: usize = 64;

/// The state of one match attempt, kept between attempts to reuse its buffers. Its capture and
/// quantifier state, and its stack's capacity, are held against the heap limit until it is
/// dropped.
pub(crate) struct Matcher<'r> {
    re: &'r Regex,
    /// Two positions per group, group 0 first: [`NONE`] where a group did not take part.
    pub(crate) caps: Vec<usize>,
    open: Vec<usize>,
    counts: Vec<u64>,
    marks: Vec<usize>,
    stack: Vec<Bt>,
    /// The stack entries held.
    stack_room: usize,
    held: HeldBytes,
    steps: u64,
}

impl<'r> Matcher<'r> {
    /// A matcher for `re`, ending the run with the heap limit when its state does not fit. Holding
    /// the state, and later the stack, can run a collection, so everything the caller still needs
    /// must be reachable from the roots while the matcher works.
    pub(crate) fn new(re: &'r Regex, interp: &mut Interp) -> JsResult<Self> {
        let state = 8usize.saturating_mul(re.groups.saturating_add(1).saturating_mul(3).saturating_add(re.loops.saturating_mul(2)));
        let mut held = HeldBytes::new(interp);
        held.add(interp, state)?;
        Ok(Self {
            re,
            caps: vec![NONE; 2 * (re.groups + 1)],
            open: vec![NONE; re.groups + 1],
            counts: vec![0; re.loops],
            marks: vec![NONE; re.loops],
            stack: Vec::new(),
            stack_room: 0,
            held,
            steps: 0,
        })
    }

    /// The bytes the matcher holds.
    #[cfg(test)]
    pub(crate) fn held_bytes(&self) -> usize {
        self.held.bytes()
    }

    fn push(&mut self, interp: &mut Interp, b: Bt) -> JsResult<()> {
        if self.stack.len() >= self.stack_room {
            self.grow(interp)?;
        }
        self.stack.push(b);
        Ok(())
    }

    /// Holds more stack entries: as many as the stack has, at least [`STACK_CHUNK`], or when those
    /// do not fit, what the heap room left holds.
    fn grow(&mut self, interp: &mut Interp) -> JsResult<()> {
        let size = size_of::<Bt>();
        let mut more = self.stack_room.max(STACK_CHUNK);
        if self.held.add(interp, more.saturating_mul(size)).is_err() {
            more = interp.limits.heap_bytes.saturating_sub(interp.heap.bytes) / size;
            if more == 0 {
                return Err(Abrupt::Limit(LimitKind::Heap));
            }
            self.held.add(interp, more * size)?;
        }
        self.stack_room = self.stack_room.saturating_add(more);
        self.stack.reserve_exact(self.stack_room - self.stack.len());
        Ok(())
    }

    fn step(&mut self, interp: &mut Interp) -> JsResult<()> {
        self.steps += 1;
        if self.steps >= FUEL_BATCH {
            let n = std::mem::take(&mut self.steps);
            use_fuel_n(interp, n)?;
        }
        Ok(())
    }

    /// Takes the fuel for the steps not yet charged.
    fn settle(&mut self, interp: &mut Interp) -> JsResult<()> {
        let n = std::mem::take(&mut self.steps);
        use_fuel_n(interp, n)
    }

    fn set_cap(&mut self, interp: &mut Interp, slot: usize, v: usize) -> JsResult<()> {
        let Some(old) = self.caps.get(slot).copied() else { return Ok(()) };
        if old != v {
            self.push(interp, Bt::Cap(slot, old))?;
            if let Some(c) = self.caps.get_mut(slot) {
                *c = v;
            }
        }
        Ok(())
    }

    /// Starts an iteration of the quantifier at `pc`, clearing the groups inside its atom. Each
    /// group cleared counts as a step.
    fn enter_rep(&mut self, interp: &mut Interp, pc: usize, pos: usize) -> JsResult<()> {
        let Some(Inst::Rep { reg, caps: (lo, hi), .. }) = self.re.insts.get(pc).copied() else { return Ok(()) };
        let reg = reg as usize;
        let old = self.marks.get(reg).copied().unwrap_or(NONE);
        self.push(interp, Bt::Mark(reg, old))?;
        if let Some(m) = self.marks.get_mut(reg) {
            *m = pos;
        }
        for g in lo as usize..hi as usize {
            self.step(interp)?;
            self.set_cap(interp, 2 * g, NONE)?;
            self.set_cap(interp, 2 * g + 1, NONE)?;
        }
        Ok(())
    }

    /// Restores the value an undo entry saved; other entries are ignored.
    fn undo(&mut self, b: Bt) {
        let (slot, old) = match b {
            Bt::Cap(s, old) => (self.caps.get_mut(s), old),
            Bt::Open(g, old) => (self.open.get_mut(g), old),
            Bt::Mark(r, old) => (self.marks.get_mut(r), old),
            Bt::Count(r, old) => {
                if let Some(c) = self.counts.get_mut(r) {
                    *c = old;
                }
                return;
            }
            _ => return,
        };
        if let Some(slot) = slot {
            *slot = old;
        }
    }

    /// Goes back to the last choice point, restoring what was changed since. False when there is
    /// none left.
    fn backtrack(&mut self, interp: &mut Interp, pc: &mut usize, pos: &mut usize) -> JsResult<bool> {
        while let Some(b) = self.stack.pop() {
            self.step(interp)?;
            match b {
                Bt::Retry { pc: p, pos: q } => {
                    (*pc, *pos) = (p, q);
                    return Ok(true);
                }
                Bt::EnterRep { pc: p, pos: q } => {
                    *pos = q;
                    self.enter_rep(interp, p, q)?;
                    *pc = p + 1;
                    return Ok(true);
                }
                Bt::Cap(..) | Bt::Open(..) | Bt::Count(..) | Bt::Mark(..) => self.undo(b),
                // A negative lookahead whose pattern failed succeeds.
                Bt::Look { pos: q, negative: true, resume } => {
                    (*pc, *pos) = (resume, q);
                    return Ok(true);
                }
                Bt::Look { negative: false, .. } => {}
            }
        }
        Ok(false)
    }

    /// Tries to match at `start` only. On success `caps` holds the match.
    pub(crate) fn match_at(&mut self, interp: &mut Interp, input: &[u16], start: usize) -> JsResult<bool> {
        let r = self.run(interp, input, start);
        self.settle(interp)?;
        r
    }

    /// The first match at `from` or after.
    pub(crate) fn search(&mut self, interp: &mut Interp, input: &[u16], from: usize) -> JsResult<bool> {
        let mut start = from;
        while start <= input.len() {
            if self.match_at(interp, input, start)? {
                return Ok(true);
            }
            start += 1;
        }
        Ok(false)
    }

    /// Clears the state for a new attempt. Each slot cleared counts as a step.
    fn reset(&mut self) {
        let slots = self.caps.len() + self.open.len() + self.counts.len() + self.marks.len();
        self.steps = self.steps.saturating_add(slots as u64);
        self.caps.fill(NONE);
        self.open.fill(NONE);
        self.counts.fill(0);
        self.marks.fill(NONE);
        self.stack.clear();
    }

    fn run(&mut self, interp: &mut Interp, input: &[u16], start: usize) -> JsResult<bool> {
        self.reset();
        let re = self.re;
        let mut pc = 0usize;
        let mut pos = start;
        let at = |pc: usize, off: i32| -> usize { pc.wrapping_add_signed(off as isize) };
        loop {
            self.step(interp)?;
            let Some(inst) = re.insts.get(pc).copied() else { return Err(Abrupt::Internal("regular expression jumped out of its code".into())) };
            let ok = match inst {
                Inst::Char(c) => {
                    let hit = input.get(pos).is_some_and(|&u| re.canon(u) == c);
                    if hit {
                        pos += 1;
                        pc += 1;
                    }
                    hit
                }
                Inst::Any => {
                    let hit = input.get(pos).is_some_and(|&u| !is_line_terminator(u));
                    if hit {
                        pos += 1;
                        pc += 1;
                    }
                    hit
                }
                Inst::Set(k) => {
                    let hit = match (input.get(pos), re.sets.get(k as usize)) {
                        (Some(&u), Some(set)) => re.set_matches(set, u),
                        _ => false,
                    };
                    if hit {
                        pos += 1;
                        pc += 1;
                    }
                    hit
                }
                Inst::LineStart => {
                    let hit = pos == 0 || (re.multiline && input.get(pos - 1).is_some_and(|&u| is_line_terminator(u)));
                    pc += 1;
                    hit
                }
                Inst::LineEnd => {
                    let hit = pos == input.len() || (re.multiline && input.get(pos).is_some_and(|&u| is_line_terminator(u)));
                    pc += 1;
                    hit
                }
                Inst::WordBoundary(want) => {
                    let a = pos > 0 && input.get(pos - 1).is_some_and(|&u| is_word_char(u));
                    let b = input.get(pos).is_some_and(|&u| is_word_char(u));
                    pc += 1;
                    (a != b) == want
                }
                Inst::Split(first, second) => {
                    self.push(interp, Bt::Retry { pc: at(pc, second), pos })?;
                    pc = at(pc, first);
                    true
                }
                Inst::Jump(off) => {
                    pc = at(pc, off);
                    true
                }
                Inst::GroupStart(g) => {
                    let g = g as usize;
                    let old = self.open.get(g).copied().unwrap_or(NONE);
                    self.push(interp, Bt::Open(g, old))?;
                    if let Some(o) = self.open.get_mut(g) {
                        *o = pos;
                    }
                    pc += 1;
                    true
                }
                Inst::GroupEnd(g) => {
                    let g = g as usize;
                    let s = self.open.get(g).copied().unwrap_or(NONE);
                    self.set_cap(interp, 2 * g, s)?;
                    self.set_cap(interp, 2 * g + 1, pos)?;
                    pc += 1;
                    true
                }
                Inst::RepInit(r) => {
                    let r = r as usize;
                    let old = self.counts.get(r).copied().unwrap_or(0);
                    self.push(interp, Bt::Count(r, old))?;
                    if let Some(c) = self.counts.get_mut(r) {
                        *c = 0;
                    }
                    pc += 1;
                    true
                }
                Inst::Rep { reg, min, max, greedy, exit, .. } => {
                    let count = self.counts.get(reg as usize).copied().unwrap_or(0);
                    let exit_pc = at(pc, exit);
                    if count >= max {
                        pc = exit_pc;
                    } else if count < min {
                        self.enter_rep(interp, pc, pos)?;
                        pc += 1;
                    } else if greedy {
                        self.push(interp, Bt::Retry { pc: exit_pc, pos })?;
                        self.enter_rep(interp, pc, pos)?;
                        pc += 1;
                    } else {
                        self.push(interp, Bt::EnterRep { pc, pos })?;
                        pc = exit_pc;
                    }
                    true
                }
                Inst::RepEnd { reg, head } => {
                    let head_pc = at(pc, head);
                    let min = match re.insts.get(head_pc) {
                        Some(Inst::Rep { min, .. }) => *min,
                        _ => return Err(Abrupt::Internal("quantifier end without its head".into())),
                    };
                    let r = reg as usize;
                    let count = self.counts.get(r).copied().unwrap_or(0);
                    // An iteration past the minimum that matched nothing fails (section
                    // 15.10.2.5, RepeatMatcher step 2.a).
                    if count >= min && self.marks.get(r).copied() == Some(pos) {
                        false
                    } else {
                        self.push(interp, Bt::Count(r, count))?;
                        if let Some(c) = self.counts.get_mut(r) {
                            *c = count + 1;
                        }
                        pc = head_pc;
                        true
                    }
                }
                Inst::Look { negative, end } => {
                    self.push(interp, Bt::Look { pos, negative, resume: at(pc, end) })?;
                    pc += 1;
                    true
                }
                Inst::LookEnd => {
                    let Some(b) = self.stack.iter().rposition(|e| matches!(e, Bt::Look { .. })) else {
                        return Err(Abrupt::Internal("lookahead end without its start".into()));
                    };
                    // Each entry read above, and below, counts as a step.
                    self.steps = self.steps.saturating_add((self.stack.len() - b) as u64);
                    let Some(Bt::Look { pos: q, negative, resume }) = self.stack.get(b).copied() else {
                        return Err(Abrupt::Internal("lookahead end without its start".into()));
                    };
                    if negative {
                        // The pattern matched, so the negative lookahead fails: restore what it
                        // changed, drop its choice points and go back past it.
                        while self.stack.len() > b + 1 {
                            if let Some(e) = self.stack.pop() {
                                self.undo(e);
                            }
                        }
                        self.stack.truncate(b);
                        false
                    } else {
                        // A lookahead is atomic: its choice points are dropped, and what it set is
                        // kept, with the entries that restore it, moved down in place.
                        let mut kept = b;
                        for r in b + 1..self.stack.len() {
                            if let Some(e) = self.stack.get(r).copied().filter(Bt::is_undo) {
                                if let Some(slot) = self.stack.get_mut(kept) {
                                    *slot = e;
                                }
                                kept += 1;
                            }
                        }
                        self.stack.truncate(kept);
                        pos = q;
                        pc = resume;
                        true
                    }
                }
                Inst::BackRef(g) => {
                    let g = g as usize;
                    let (s, e) = (self.caps.get(2 * g).copied().unwrap_or(NONE), self.caps.get(2 * g + 1).copied().unwrap_or(NONE));
                    pc += 1;
                    if s == NONE || e == NONE || e < s {
                        true
                    } else {
                        let len = e - s;
                        let mut hit = pos + len <= input.len();
                        // The units are compared 16 at a time, and each 16, or fewer at the end,
                        // count as a step taken before they are compared.
                        let mut k = 0;
                        while hit && k < len {
                            let end = len.min(k + 16);
                            self.step(interp)?;
                            hit = (k..end).all(|i| match (input.get(s + i), input.get(pos + i)) {
                                (Some(&a), Some(&b)) => re.canon(a) == re.canon(b),
                                _ => false,
                            });
                            k = end;
                        }
                        if hit {
                            pos += len;
                        }
                        hit
                    }
                }
                Inst::Match => {
                    self.set_cap(interp, 0, start)?;
                    self.set_cap(interp, 1, pos)?;
                    return Ok(true);
                }
            };
            if !ok && !self.backtrack(interp, &mut pc, &mut pos)? {
                return Ok(false);
            }
        }
    }
}
