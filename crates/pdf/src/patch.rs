//! Edits to a PDF the export wrote (uncompressed objects, one cross-reference table starting at
//! object 0): bytes replaced or inserted inside objects, new objects and trailer entries. The
//! cross-reference table is written again to match.

use crate::PdfError;
use crate::lab_spot::{find, rfind, xref_offset};

/// The cross-reference table of a written PDF and its trailer.
pub(crate) struct Xref {
    /// Where the table (`xref`) starts.
    at: usize,
    /// Where its first 20-byte entry starts.
    entries: usize,
    /// Number of entries (the trailer's `/Size`).
    count: usize,
    /// The trailer dictionary, `<<` to `>>` included.
    trailer: (usize, usize),
}

/// Most bytes of a number in PDF syntax read here.
const MAX_DIGITS: usize = 10;

/// The unsigned number `pdf` holds at `at` (after white space), and where it ends.
pub(crate) fn number(pdf: &[u8], mut at: usize) -> Option<(usize, usize)> {
    while pdf.get(at).is_some_and(u8::is_ascii_whitespace) {
        at += 1;
    }
    let digits = pdf.get(at..)?.iter().take(MAX_DIGITS + 1).take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 || digits > MAX_DIGITS {
        return None;
    }
    Some((std::str::from_utf8(pdf.get(at..at + digits)?).ok()?.parse().ok()?, at + digits))
}

impl Xref {
    /// The table `startxref` points at; `None` for files written another way.
    pub fn read(pdf: &[u8]) -> Option<Self> {
        let at = xref_offset(pdf)?;
        let (first, after) = number(pdf, at + 4)?;
        let (count, after) = number(pdf, after)?;
        let entries = find(pdf, b"\n", after)? + 1;
        let table_end = entries.checked_add(count.checked_mul(20)?)?;
        if first != 0 || count == 0 || table_end > pdf.len() {
            return None;
        }
        let start = find(pdf, b"<<", find(pdf, b"trailer", table_end)?)?;
        let end = rfind(pdf.get(start..rfind(pdf, b"startxref")?)?, b">>")? + start + 2;
        Some(Self { at, entries, count, trailer: (start, end) })
    }

    /// Number of entries (objects `0..count`).
    pub fn count(&self) -> usize {
        self.count
    }

    /// Where in-use object `n` starts.
    pub fn offset(&self, pdf: &[u8], n: u32) -> Option<usize> {
        let e = self.entries + 20 * usize::try_from(n).ok().filter(|n| *n < self.count)?;
        let entry = pdf.get(e..e + 20)?;
        (entry.get(17) == Some(&b'n')).then_some(())?;
        number(entry, 0).map(|(off, _)| off)
    }

    /// The in-use objects: number and offset, in number order.
    pub fn objects(&self, pdf: &[u8]) -> Vec<(u32, usize)> {
        (1..u32::try_from(self.count).unwrap_or(u32::MAX)).filter_map(|n| Some((n, self.offset(pdf, n)?))).collect()
    }

    /// Where the table (`xref`) starts.
    pub fn at(&self) -> usize {
        self.at
    }

    /// Where the trailer dictionary starts (its `<<`).
    pub fn trailer_at(&self) -> usize {
        self.trailer.0
    }

    /// The object the trailer's `key` (`b"/Root"`) refers to.
    pub fn trailer_ref(&self, pdf: &[u8], key: &[u8]) -> Option<u32> {
        let dict = pdf.get(self.trailer.0..self.trailer.1)?;
        let at = find(dict, key, 0)? + key.len();
        // A key that only starts with `key` (`/Info` in `/InfoX`) isn't it.
        if dict.get(at).is_some_and(|b| b.is_ascii_alphanumeric()) {
            return None;
        }
        u32::try_from(number(dict, at)?.0).ok()
    }

    /// The contents of the dictionary object `n` is (just after its `<<`, up to its `endobj`).
    pub fn dict(&self, pdf: &[u8], n: u32) -> Option<(usize, usize)> {
        let off = self.offset(pdf, n)?;
        let (num, after) = number(pdf, off)?;
        let head = pdf.get(after..after + 6)?;
        if num != n as usize || head != b" 0 obj" {
            return None;
        }
        let mut at = after + 6;
        while pdf.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        (pdf.get(at..at + 2)? == b"<<").then_some(())?;
        Some((at + 2, find(pdf, b"endobj", at)?))
    }
}

/// A replacement of bytes `start..end` of the file.
type Edit = (usize, usize, Vec<u8>);

/// Where byte offsets of the old file land in the new one, for edits sorted by start that do not
/// overlap, whose ends therefore never decrease.
struct Moves {
    /// Each edit's end, in order.
    ends: Vec<usize>,
    /// The change in length made by the edits up to and including each one.
    shifts: Vec<isize>,
}

impl Moves {
    /// `None` when a change in length doesn't fit an `isize`.
    fn new(edits: &[Edit]) -> Option<Self> {
        let mut total: isize = 0;
        let mut shifts = Vec::with_capacity(edits.len());
        for (start, end, new) in edits {
            let removed = isize::try_from(end.checked_sub(*start)?).ok()?;
            total = total.checked_add(isize::try_from(new.len()).ok()?)?.checked_sub(removed)?;
            shifts.push(total);
        }
        Some(Self { ends: edits.iter().map(|e| e.1).collect(), shifts })
    }

    /// Where `off` lands: moved by every edit that ends at or before it.
    fn at(&self, off: usize) -> Option<usize> {
        match self.ends.partition_point(|&end| end <= off) {
            0 => Some(off),
            n => off.checked_add_signed(*self.shifts.get(n - 1)?),
        }
    }
}

/// `off` as the ten digits of a cross-reference entry; `None` past what ten digits hold.
fn entry_offset(off: usize) -> Option<String> {
    (u64::try_from(off).ok()? < 10_000_000_000).then(|| format!("{off:010}"))
}

/// The edits of one PDF, applied together by [`Self::apply`].
pub(crate) struct Patch {
    edits: Vec<Edit>,
    /// New objects' bodies, numbered from the table's size on.
    objects: Vec<Vec<u8>>,
    /// Entries added to the trailer dictionary.
    trailer: Vec<u8>,
    size: usize,
}

impl Patch {
    pub fn new(xref: &Xref) -> Self {
        Self { edits: vec![], objects: vec![], trailer: vec![], size: xref.count }
    }

    /// Replace bytes `start..end` (inside the objects) with `with`; `start == end` inserts.
    pub fn replace(&mut self, start: usize, end: usize, with: Vec<u8>) {
        self.edits.push((start, end, with));
    }

    /// Add an object of `body` (a dictionary, array or stream with its dictionary) → its number.
    pub fn add_object(&mut self, body: Vec<u8>) -> u32 {
        self.objects.push(body);
        u32::try_from(self.size + self.objects.len() - 1).unwrap_or(u32::MAX)
    }

    /// Add `entry` (`/Info 12 0 R`) to the trailer dictionary.
    pub fn trailer_entry(&mut self, entry: &[u8]) {
        self.trailer.extend_from_slice(entry);
    }

    /// `pdf` with the edits, new objects and trailer entries, and a cross-reference table that
    /// matches. Refused when an edit overlaps another or reaches past the objects, or when an
    /// object would start past the ten digits a cross-reference entry gives its offset in.
    pub fn apply(mut self, pdf: &[u8], xref: &Xref) -> Result<Vec<u8>, PdfError> {
        let bad = |why: &str| PdfError::Write(format!("can't edit the written PDF: {why}"));
        self.edits.sort_by_key(|e| (e.0, e.1));
        if self.edits.windows(2).any(|w| w[0].1 > w[1].0) || self.edits.iter().any(|e| e.0 > e.1 || e.1 > xref.at) {
            return Err(bad("edits overlap"));
        }
        let edits = &self.edits;
        let moves = Moves::new(edits).ok_or_else(|| bad("edits too large"))?;
        let too_far = || bad("an object lies past what the cross-reference table holds");
        let added: usize = self.edits.iter().map(|e| e.2.len()).sum::<usize>() + self.objects.iter().map(|o| o.len() + 32).sum::<usize>();
        let mut out = Vec::with_capacity(pdf.len() + added + 64);
        let mut at = 0;
        for (start, end, new) in edits.iter() {
            out.extend_from_slice(pdf.get(at..*start).ok_or_else(|| bad("bad edit"))?);
            out.extend_from_slice(new);
            at = *end;
        }
        out.extend_from_slice(pdf.get(at..xref.at).ok_or_else(|| bad("bad edit"))?);
        let mut offsets = Vec::with_capacity(self.objects.len());
        for (i, body) in self.objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", self.size + i).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        // The table: the old entries moved (free ones as they are), then the new objects'.
        let table = out.len();
        let size = self.size + self.objects.len();
        out.extend_from_slice(format!("xref\n0 {size}\n").as_bytes());
        let old = pdf.get(xref.entries..xref.entries + 20 * xref.count).ok_or_else(|| bad("bad cross-reference table"))?;
        for e in old.chunks(20) {
            match number(e, 0) {
                Some((off, _)) if e.get(17) == Some(&b'n') => {
                    out.extend_from_slice(moves.at(off).and_then(entry_offset).ok_or_else(too_far)?.as_bytes());
                    out.extend_from_slice(e.get(10..).unwrap_or_default());
                }
                _ => out.extend_from_slice(e),
            }
        }
        let eol = old.get(18..20).unwrap_or(b"\r\n");
        for off in offsets {
            out.extend_from_slice(entry_offset(off).ok_or_else(too_far)?.as_bytes());
            out.extend_from_slice(b" 00000 n");
            out.extend_from_slice(eol);
        }
        // The trailer, with its size and the new entries.
        let (ts, te) = xref.trailer;
        let dict = pdf.get(ts..te.saturating_sub(2)).ok_or_else(|| bad("bad trailer"))?;
        let size_at = find(dict, b"/Size", 0).ok_or_else(|| bad("the trailer has no size"))? + 5;
        let (_, size_end) = number(dict, size_at).ok_or_else(|| bad("the trailer has no size"))?;
        let (Some(head), Some(tail)) = (dict.get(..size_at), dict.get(size_end..)) else { return Err(bad("bad trailer")) };
        out.extend_from_slice(b"trailer\n");
        out.extend_from_slice(head);
        out.extend_from_slice(format!(" {size}").as_bytes());
        out.extend_from_slice(tail);
        out.extend_from_slice(&self.trailer);
        out.extend_from_slice(format!(">>\nstartxref\n{table}\n%%EOF").as_bytes());
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// A PDF written as the export writes one: objects `<< /N n >>` numbered from 1, those `free`
    /// picks left out with a free entry, then one cross-reference table and the trailer.
    fn written(count: usize, free: impl Fn(usize) -> bool) -> Vec<u8> {
        let mut pdf = b"%PDF-1.7\n".to_vec();
        let mut entries = b"0000000000 65535 f\r\n".to_vec();
        for n in 1..=count {
            if free(n) {
                entries.extend_from_slice(b"0000000000 00001 f\r\n");
                continue;
            }
            entries.extend(format!("{:010} 00000 n\r\n", pdf.len()).bytes());
            pdf.extend(format!("{n} 0 obj\n<< /N {n} >>\nendobj\n").bytes());
        }
        let at = pdf.len();
        pdf.extend(format!("xref\n0 {}\n", count + 1).bytes());
        pdf.extend(entries);
        pdf.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{at}\n%%EOF", count + 1).bytes());
        pdf
    }

    /// A small linear congruential generator, so every run makes the same edits.
    struct Rng(u64);

    impl Rng {
        /// A number in `0..n` (0 when `n` is 0).
        fn below(&mut self, n: usize) -> usize {
            self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            usize::try_from(self.0 >> 33).unwrap() % n.max(1)
        }

        fn bytes(&mut self) -> Vec<u8> {
            (0..self.below(9)).map(|i| b'a' + u8::try_from(i).unwrap()).collect()
        }
    }

    /// About `wanted` edits that don't overlap, in shuffled order, at random places among the
    /// objects and at object starts: replacements that grow, shrink or remove bytes, edits that end
    /// at an object's start, and inserts, several of them at one place. None when `wanted` is 0.
    fn edits(pdf: &[u8], xref: &Xref, rng: &mut Rng, wanted: usize) -> Vec<Edit> {
        if wanted == 0 {
            return vec![];
        }
        let first = b"%PDF-1.7\n".len();
        let mut points: Vec<usize> = xref.objects(pdf).into_iter().map(|(_, off)| off).collect();
        points.extend((0..wanted).map(|_| first + rng.below(xref.at() - first + 1)));
        points.push(xref.at());
        points.sort_unstable();
        points.dedup();
        let mut out = vec![];
        for w in points.windows(2) {
            let (p, q) = (w[0], w[1]);
            match rng.below(6) {
                0 => {}
                1 => out.push((p, q, rng.bytes())),
                2 => out.push((p, p, rng.bytes())),
                3 => {
                    out.push((p, p, rng.bytes()));
                    out.push((p, p, rng.bytes()));
                    out.push((p, q.min(p + rng.below(4)), rng.bytes()));
                }
                4 => out.push((p + (q - p) / 2, q, rng.bytes())),
                _ => out.push((q, q, rng.bytes())),
            }
        }
        for i in (1..out.len()).rev() {
            out.swap(i, rng.below(i + 1));
        }
        out
    }

    fn patch(xref: &Xref, edits: &[Edit], objects: bool) -> Patch {
        let mut patch = Patch::new(xref);
        for (start, end, with) in edits {
            patch.replace(*start, *end, with.clone());
        }
        if objects {
            patch.add_object(b"<< /Added 1 >>".to_vec());
            patch.trailer_entry(b" /Info 2 0 R");
        }
        patch
    }

    /// Where `off` lands, by walking every edit: how [`Patch::apply`] found it before [`Moves`].
    fn scan(edits: &[Edit], off: usize) -> usize {
        edits.iter().filter(|e| e.1 <= off).fold(off as isize, |o, e| o + e.2.len() as isize - (e.1 - e.0) as isize) as usize
    }

    /// [`Patch::apply`] as it was written before [`Moves`], walking every edit for every object.
    fn apply_by_scan(mut patch: Patch, pdf: &[u8], xref: &Xref) -> Result<Vec<u8>, PdfError> {
        let bad = |why: &str| PdfError::Write(format!("can't edit the written PDF: {why}"));
        patch.edits.sort_by_key(|e| (e.0, e.1));
        if patch.edits.windows(2).any(|w| w[0].1 > w[1].0) || patch.edits.iter().any(|e| e.0 > e.1 || e.1 > xref.at) {
            return Err(bad("edits overlap"));
        }
        let edits = &patch.edits;
        let moved = |off: usize| -> usize { scan(edits, off) };
        let added: usize = patch.edits.iter().map(|e| e.2.len()).sum::<usize>() + patch.objects.iter().map(|o| o.len() + 32).sum::<usize>();
        let mut out = Vec::with_capacity(pdf.len() + added + 64);
        let mut at = 0;
        for (start, end, new) in edits.iter() {
            out.extend_from_slice(pdf.get(at..*start).ok_or_else(|| bad("bad edit"))?);
            out.extend_from_slice(new);
            at = *end;
        }
        out.extend_from_slice(pdf.get(at..xref.at).ok_or_else(|| bad("bad edit"))?);
        let mut offsets = Vec::with_capacity(patch.objects.len());
        for (i, body) in patch.objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", patch.size + i).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let table = out.len();
        let size = patch.size + patch.objects.len();
        out.extend_from_slice(format!("xref\n0 {size}\n").as_bytes());
        let old = pdf.get(xref.entries..xref.entries + 20 * xref.count).ok_or_else(|| bad("bad cross-reference table"))?;
        for e in old.chunks(20) {
            match number(e, 0) {
                Some((off, _)) if e.get(17) == Some(&b'n') => {
                    out.extend_from_slice(format!("{:010}", moved(off)).as_bytes());
                    out.extend_from_slice(e.get(10..).unwrap_or_default());
                }
                _ => out.extend_from_slice(e),
            }
        }
        let eol = old.get(18..20).unwrap_or(b"\r\n");
        for off in offsets {
            out.extend_from_slice(format!("{off:010} 00000 n").as_bytes());
            out.extend_from_slice(eol);
        }
        let (ts, te) = xref.trailer;
        let dict = pdf.get(ts..te.saturating_sub(2)).ok_or_else(|| bad("bad trailer"))?;
        let size_at = find(dict, b"/Size", 0).ok_or_else(|| bad("the trailer has no size"))? + 5;
        let (_, size_end) = number(dict, size_at).ok_or_else(|| bad("the trailer has no size"))?;
        let (Some(head), Some(tail)) = (dict.get(..size_at), dict.get(size_end..)) else { return Err(bad("bad trailer")) };
        out.extend_from_slice(b"trailer\n");
        out.extend_from_slice(head);
        out.extend_from_slice(format!(" {size}").as_bytes());
        out.extend_from_slice(tail);
        out.extend_from_slice(&patch.trailer);
        out.extend_from_slice(format!(">>\nstartxref\n{table}\n%%EOF").as_bytes());
        Ok(out)
    }

    #[test]
    fn every_offset_moves_by_the_edits_that_end_at_or_before_it() {
        let mut rng = Rng(7661);
        for round in 0..100 {
            let pdf = written(1 + rng.below(30), |n| n % 7 == 3);
            let xref = Xref::read(&pdf).unwrap();
            let mut edits = edits(&pdf, &xref, &mut rng, [0, 1, 3, 20, 80][round % 5]);
            edits.sort_by_key(|e| (e.0, e.1));
            let moves = Moves::new(&edits).unwrap();
            for off in 0..=pdf.len() {
                assert_eq!(moves.at(off), Some(scan(&edits, off)), "round {round}, offset {off}, edits {edits:?}");
            }
        }
    }

    #[test]
    fn apply_writes_the_bytes_a_walk_of_every_edit_wrote() {
        let mut rng = Rng(1378);
        for round in 0..300 {
            let pdf = written(1 + rng.below(40), |n| n % 7 == 3);
            let xref = Xref::read(&pdf).unwrap();
            let edits = edits(&pdf, &xref, &mut rng, [0, 1, 3, 20, 80][round % 5]);
            let objects = round % 3 == 0;
            let fast = patch(&xref, &edits, objects).apply(&pdf, &xref).unwrap();
            let walked = apply_by_scan(patch(&xref, &edits, objects), &pdf, &xref).unwrap();
            assert_eq!(String::from_utf8_lossy(&fast), String::from_utf8_lossy(&walked), "round {round}");
        }
    }

    #[test]
    fn every_entry_points_at_its_object_after_the_edits() {
        let pdf = written(30, |n| n % 7 == 3);
        let xref = Xref::read(&pdf).unwrap();
        let mut patch = Patch::new(&xref);
        for (n, off) in xref.objects(&pdf) {
            let (at, end) = xref.dict(&pdf, n).unwrap();
            match n % 4 {
                0 => patch.replace(at, at, b" /Grown [1 2 3]".to_vec()),
                1 => patch.replace(at, end, b">>\n".to_vec()),
                // An insert at an object's start goes before the object.
                2 => patch.replace(off, off, b"% before\n".to_vec()),
                _ => {}
            }
        }
        let added = patch.add_object(b"<< /Added 1 >>".to_vec());
        let out = patch.apply(&pdf, &xref).unwrap();
        let after = Xref::read(&out).unwrap();
        assert_eq!((after.count(), added), (32, 31));
        for n in 1..32u32 {
            let Some(off) = after.offset(&out, n) else {
                assert!(n % 7 == 3, "object {n} is in use");
                let e = xref.entries + 20 * n as usize;
                assert_eq!(out[after.entries + 20 * n as usize..][..20], pdf[e..e + 20], "free entry {n}");
                continue;
            };
            assert!(out[off..].starts_with(format!("{n} 0 obj\n").as_bytes()), "object {n} at {off}");
        }
    }

    #[test]
    fn an_object_moved_past_ten_digits_is_refused() {
        let mut pdf = written(2, |_| false);
        let entry = Xref::read(&pdf).unwrap().entries + 40;
        // Object 2's entry gives the last offset ten digits hold.
        pdf[entry..entry + 10].copy_from_slice(b"9999999999");
        let xref = Xref::read(&pdf).unwrap();
        let out = Patch::new(&xref).apply(&pdf, &xref).unwrap();
        assert!(find(&out, b"9999999999 00000 n\r\n", 0).is_some());
        let mut patch = Patch::new(&xref);
        patch.replace(9, 9, b"%\n".to_vec());
        let refused = patch.apply(&pdf, &xref);
        assert!(matches!(&refused, Err(PdfError::Write(why)) if why.contains("cross-reference table holds")), "{refused:?}");
    }

    /// CPU time this thread has used, where the system reports it (`/proc` on Linux).
    fn thread_cpu() -> Option<Duration> {
        let stat = std::fs::read_to_string("/proc/thread-self/schedstat").ok()?;
        stat.split_whitespace().next()?.parse().ok().map(Duration::from_nanos)
    }

    /// How long `f` takes, in thread CPU time where the system reports it, else wall time.
    fn time<T>(f: impl FnOnce() -> T) -> (Duration, T) {
        let (cpu, wall) = (thread_cpu(), Instant::now());
        let out = f();
        (thread_cpu().zip(cpu).map_or_else(|| wall.elapsed(), |(b, a)| b.saturating_sub(a)), out)
    }

    fn median(mut runs: Vec<Duration>) -> Duration {
        runs.sort_unstable();
        runs[runs.len() / 2]
    }

    /// `cargo test --release -p vectorcraft-pdf --lib patch::tests::bench -- --ignored --nocapture`.
    /// The edits are those of forms in overprint: two inside each form's stream (its `/Length` and
    /// data) and an insert into its dictionary, for `n` forms among `n + 20` objects. Each run
    /// applies the patch `reps` times, so that small files take long enough for the clock.
    #[test]
    #[ignore = "timing; run explicitly"]
    fn bench_apply_with_many_edits() {
        println!("clock: {}", if thread_cpu().is_some() { "thread CPU time" } else { "wall time" });
        println!("| forms | objects | edits | walk every edit (median of 7) | binary search (median of 7) |");
        println!("| ---: | ---: | ---: | ---: | ---: |");
        for n in [100, 1_000, 5_000, 20_000] {
            let pdf = written(n + 20, |_| false);
            let xref = Xref::read(&pdf).unwrap();
            let mut edits = vec![];
            for (obj, _) in xref.objects(&pdf).into_iter().take(n) {
                let (at, _) = xref.dict(&pdf, obj).unwrap();
                edits.push((at, at + 1, b" /Length 120 ".to_vec()));
                edits.push((at + 1, at + 3, b"/Data".to_vec()));
                edits.push((at + 3, at + 3, b" /Resources << /ExtGState << /GS0 9 0 R >> >>".to_vec()));
            }
            let reps = (200_000_000 / (3 * n * n)).clamp(1, 2_000);
            let (mut walked, mut searched) = (vec![], vec![]);
            for _ in 0..7 {
                let (t, old) = time(|| (0..reps).fold(None, |_, _| apply_by_scan(patch(&xref, &edits, false), &pdf, &xref).ok()));
                walked.push(t);
                let (t, new) = time(|| (0..reps).fold(None, |_, _| patch(&xref, &edits, false).apply(&pdf, &xref).ok()));
                searched.push(t);
                assert!(old.is_some() && old == new, "{n} forms: the files differ");
            }
            let ms = |d: Duration| d.as_secs_f64() * 1000.0 / reps as f64;
            println!("| {n} | {} | {} | {:.3} ms | {:.3} ms |", n + 20, edits.len(), ms(median(walked)), ms(median(searched)));
        }
    }
}
