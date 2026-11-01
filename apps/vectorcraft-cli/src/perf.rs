//! `vectorcraft-cli perf`: the performance budgets (plan §4) measured on a synthetic document.
//!
//! Each row is the median of several runs. Timings are wall-clock: on a busy machine (a load
//! average above three quarters of the cores, or on Windows more than three quarters of the
//! cores busy over a second) they are noise, and the report says so.

use std::time::Instant;

use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, hit};
use vectorcraft_geom::{Affine, Point, Rect, shapes};
use vectorcraft_render::{RenderOptions, Renderer};

/// Deterministic xorshift in 0..1.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % 1_000_000) as f64 / 1_000_000.0
    }
}

/// The synthetic document's page, in points.
const SYNTHETIC_PAGE: (f64, f64) = (1600.0, 1200.0);

/// `n` mixed paths (ellipses with strokes, translucent rectangles, stroked stars) on a 1600×1200 page.
pub fn synthetic(n: usize) -> Result<Document, String> {
    let mut d = Document::new(SYNTHETIC_PAGE.0, SYNTHETIC_PAGE.1);
    let l = d.layers.first().map(|l| l.id);
    let mut r = Rng(42);
    for i in 0..n {
        let (x, y, s) = (r.next() * SYNTHETIC_PAGE.0, r.next() * SYNTHETIC_PAGE.1, 3.0 + r.next() * 22.0);
        let c = Color::rgb(r.next() as f32, r.next() as f32, r.next() as f32);
        let id = d.alloc_id();
        let mut node = match i % 3 {
            0 => Node::path(
                id,
                shapes::ellipse(Rect::from_center_size(Point::new(x, y), (2.0 * s, 2.0 * s))),
                Appearance::basic(Paint::solid(c), Paint::solid(Color::BLACK), 0.5),
            ),
            1 => Node::path(id, shapes::rectangle(Rect::new(x, y, x + 2.0 * s, y + s)), Appearance::basic(Paint::solid(c), Paint::None, 0.0)),
            _ => Node::path(id, shapes::star(Point::new(x, y), s, s / 2.0, 5, 0.0), Appearance::basic(Paint::None, Paint::solid(c), 2.0)),
        };
        if i % 3 == 1 {
            node.opacity = 0.8;
        }
        d.insert(l, usize::MAX, node).map_err(|e| format!("building the synthetic document: {e}"))?;
    }
    Ok(d)
}

fn median_ms(runs: usize, mut f: impl FnMut()) -> f64 {
    let mut v: Vec<f64> = (0..runs)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    v.sort_by(f64::total_cmp);
    v.get(v.len() / 2).copied().unwrap_or(0.0)
}

/// 1-minute load average (macOS / Linux), if available.
#[cfg(not(windows))]
fn load_average() -> Option<f64> {
    let out = std::process::Command::new("sysctl").args(["-n", "vm.loadavg"]).output().ok().filter(|o| o.status.success());
    let text = match out {
        Some(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        None => std::fs::read_to_string("/proc/loadavg").ok()?,
    };
    text.split_whitespace().map(|t| t.trim_matches(|c| c == '{' || c == '}')).find_map(|t| t.parse().ok())
}

/// The cores busy on average over one second (Windows), if available: the share of the
/// processors' time that `GetSystemTimes` reports as kernel or user time but not as idle time,
/// times `cores`. A load average also counts the threads waiting for a core.
#[cfg(windows)]
fn busy_cores(cores: usize) -> Option<f64> {
    let times = || winsafe::GetSystemTimes().ok().map(|(idle, kernel, user)| [idle, kernel, user].map(u64::from));
    let before = times()?;
    std::thread::sleep(std::time::Duration::from_secs(1));
    busy_share(before, times()?).map(|share| share * cores as f64)
}

/// The share of the processors' time between two `[idle, kernel, user]` readings of
/// `GetSystemTimes` that was busy (kernel time includes idle time). It is None when no time
/// passed, a time went back, or the idle time exceeds the kernel and user time.
#[cfg(any(windows, test))]
fn busy_share([idle0, kernel0, user0]: [u64; 3], [idle1, kernel1, user1]: [u64; 3]) -> Option<f64> {
    let total = kernel1.checked_sub(kernel0)?.checked_add(user1.checked_sub(user0)?)?;
    let busy = total.checked_sub(idle1.checked_sub(idle0)?)?;
    (total > 0).then(|| busy as f64 / total as f64)
}

/// Milliseconds per pointer move of a Direct Selection drag of a handle, then of a segment, of a
/// 5,000-anchor wave of smooth anchors added on top of the synthetic `n`-path document.
fn edit_drags(n: usize) -> Result<(f64, f64), String> {
    use vectorcraft_engine::tools::{Mods, PointerEvent, PointerKind};
    use vectorcraft_geom::{Anchor, PathData, SubPath, Vec2};
    let mut doc = synthetic(n)?;
    let l = doc.layers.first().map(|l| l.id);
    let id = doc.alloc_id();
    // Anchors 2 pt apart, up and down by 20 pt, their handles 0.6 pt long.
    let anchor = |i: usize| {
        let p = Point::new(i as f64 * 2.0, if i.is_multiple_of(2) { 500.0 } else { 520.0 });
        Anchor::smooth(p, p + Vec2::new(0.6, 0.0))
    };
    let path = PathData::single(SubPath::new((0..5000).map(anchor).collect(), false));
    doc.insert(l, usize::MAX, Node::path(id, path, Appearance::default_art())).map_err(|e| e.to_string())?;
    let mut s = vectorcraft_engine::Session::new();
    s.add_document(doc, None);
    let view = vectorcraft_engine::ViewInfo { zoom: 8.0, ..Default::default() };
    let err = |e: vectorcraft_engine::EngineError| e.to_string();
    let mut drag = |down: Point| -> Result<f64, String> {
        let ev = |kind, y: f64| PointerEvent { kind, pos: down + Vec2::new(0.0, y), mods: Mods::default(), backquote: false, pressure: 1.0 };
        s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 400]]})).map_err(err)?;
        s.select_tool("directSelection", view).map_err(err)?;
        let undo = s.doc().map_err(err)?.history.undo.len();
        s.pointer(&ev(PointerKind::Down, 0.0), view).map_err(err)?;
        let mut y = 0.0;
        let ms = median_ms(9, || {
            y += 3.0;
            // A failed move shows as the missing undo step below.
            let _ = s.pointer(&ev(PointerKind::Drag, y), view);
        });
        s.pointer(&ev(PointerKind::Up, y), view).map_err(err)?;
        if s.doc().map_err(err)?.history.undo.len() != undo + 1 {
            return Err(format!("the drag from {down:?} made no undo step"));
        }
        Ok(ms)
    };
    // Anchor 400 is at (800, 500): its outgoing handle's end, then its segment to anchor 401.
    let handle = drag(Point::new(800.6, 500.0))?;
    let segment = drag(Point::new(801.0, 510.0))?;
    Ok((handle, segment))
}

/// Resident memory of this process in MB (`ps`), if available, with the most it has held so far
/// on Linux (`VmHWM` in `/proc/self/status`). The peak is None on macOS, which has no `/proc`.
#[cfg(not(windows))]
fn rss_mb() -> Option<(f64, Option<f64>)> {
    let pid = std::process::id().to_string();
    let out = std::process::Command::new("ps").args(["-o", "rss=", "-p", &pid]).output().ok().filter(|o| o.status.success())?;
    let peak = std::fs::read_to_string("/proc/self/status").ok().and_then(|status| vm_hwm_mb(&status));
    String::from_utf8_lossy(&out.stdout).trim().parse::<f64>().ok().map(|kb| (kb / 1024.0, peak))
}

/// The peak resident set size in MB from the `VmHWM` line of a `/proc/<pid>/status` text, which
/// gives it in kB. It is None when the text has no `VmHWM` line in kB.
#[cfg(any(not(windows), test))]
fn vm_hwm_mb(status: &str) -> Option<f64> {
    let kb = status.lines().find_map(|line| line.strip_prefix("VmHWM:"))?.trim().strip_suffix("kB")?.trim().parse::<f64>().ok()?;
    Some(kb / 1024.0)
}

/// The working set of this process in MB (`GetProcessMemoryInfo`), if available: its pages that
/// are in physical memory, the figure `ps` reports as resident memory elsewhere. The same call
/// gives the peak working set, the most the working set has held so far.
#[cfg(windows)]
fn rss_mb() -> Option<(f64, Option<f64>)> {
    let counters = winsafe::HPROCESS::GetCurrentProcess().GetProcessMemoryInfo().ok()?;
    let mb = |bytes: usize| bytes as f64 / (1024.0 * 1024.0);
    Some((mb(counters.WorkingSetSize), Some(mb(counters.PeakWorkingSetSize))))
}

/// A tracked row of `--deep`: the work's name and its median in ms. Names are padded to 54
/// characters.
fn tracked_row(name: &str, ms: f64) -> String {
    format!("  {name:<54} {ms:>10.2} ms")
}

/// A memory row of `--deep`: the reading's name and value in MB, then the peak so far when
/// there is one. Names are padded to 56 characters.
fn memory_row(name: &str, mb: f64, peak: Option<f64>) -> String {
    let peak = peak.map_or_else(String::new, |peak| format!("  peak {peak:>8.0} MB"));
    format!("  {name:<56} {mb:>8.0} MB{peak}")
}

/// A row of the budget table: the budget's name, the median and the budget in ms, then "ok" or
/// "OVER". Names are padded to 42 characters, as in the table's header.
fn budget_row(name: &str, ms: f64, budget: f64, ok: bool) -> String {
    format!("  {name:<42} {ms:>8.2} ms {budget:>7.0} ms  {}", if ok { "ok" } else { "OVER" })
}

/// `--deep`: tracked rows (no budget yet) for work the budgets above don't reach: editing a whole
/// selection, live effects, every Pathfinder operation, typing in a long story, and memory, with
/// the peak so far where `rss_mb` reads one.
fn deep(n: usize) -> Result<(), String> {
    let err = |e: vectorcraft_engine::EngineError| e.to_string();
    let mut rows: Vec<(String, f64)> = vec![];
    let mut mem: Vec<(String, f64, Option<f64>)> = vec![];
    let mut note = |what: &str| {
        if let Some((m, peak)) = rss_mb() {
            mem.push((what.to_string(), m, peak));
        }
    };
    note("start");
    // Editing every object of an n-path document.
    let mut s = vectorcraft_engine::Session::new();
    s.add_document(synthetic(n)?, None);
    note("document built");
    rows.push((format!("select all, {n} paths"), median_ms(3, || drop(s.execute("select.all", &json!({}))))));
    rows.push((format!("nudge the selection ({n} paths)"), median_ms(5, || drop(s.execute("object.move", &json!({"dx": 1, "dy": 0}))))));
    rows.push((format!("undo a nudge ({n} paths)"), median_ms(5, || drop(s.execute("edit.undo", &json!({}))))));
    rows.push((format!("redo a nudge ({n} paths)"), median_ms(5, || drop(s.execute("edit.redo", &json!({}))))));
    // Dragging the whole selection with the Selection tool: one pointer move (the preview).
    {
        use vectorcraft_engine::tools::{Mods, PointerEvent, PointerKind};
        use vectorcraft_geom::Vec2;
        let view = vectorcraft_engine::ViewInfo { zoom: 1.0, ..Default::default() };
        let first = s.doc().map_err(err)?.selection.objects.first().copied();
        let at = first.and_then(|id| s.doc().ok()?.doc.node(id)?.geometric_bounds()).map(|b| b.center());
        if let Some(down) = at {
            let ev = |kind, dx: f64| PointerEvent { kind, pos: down + Vec2::new(dx, 0.0), mods: Mods::default(), backquote: false, pressure: 1.0 };
            s.select_tool("selection", view).map_err(err)?;
            s.pointer(&ev(PointerKind::Down, 0.0), view).map_err(err)?;
            let mut dx = 0.0;
            rows.push((
                format!("drag the selection, per move ({n} paths)"),
                median_ms(5, || {
                    dx += 2.0;
                    let _ = s.pointer(&ev(PointerKind::Drag, dx), view);
                }),
            ));
            s.pointer(&ev(PointerKind::Up, dx), view).map_err(err)?;
        }
    }
    for _ in 0..40 {
        s.execute("object.move", &json!({"dx": 1, "dy": 0})).map_err(err)?;
    }
    note("after 45 whole-selection edits");
    drop(s);
    // Live effects on 2,000 paths: render a page whose every object has the effect.
    let opts = RenderOptions::default();
    for (fx, params) in [("path.offsetPath", json!({"offset": 3})), ("distort.roughen", json!({"size": 5, "relative": true}))] {
        let mut s = vectorcraft_engine::Session::new();
        s.add_document(synthetic(2000)?, None);
        s.execute("select.all", &json!({})).map_err(err)?;
        s.execute("effect.apply", &json!({"effect": fx, "params": params})).map_err(err)?;
        let doc = s.doc().map_err(err)?.doc.clone();
        let mut r = Renderer::new();
        r.render(&doc, 2880, 1800, Affine::scale(1.6), &opts);
        rows.push((format!("render, 2,000 paths with {fx} (warm)"), median_ms(5, || drop(r.render(&doc, 2880, 1800, Affine::scale(1.6), &opts)))));
    }
    note("after live effects");
    // Every Pathfinder operation on 300 overlapping paths.
    for op in ["unite", "intersect", "exclude", "minusFront", "minusBack", "divide", "trim", "merge", "crop", "outline"] {
        let ms = median_ms(1, || {
            let mut s = vectorcraft_engine::Session::new();
            if let Ok(d) = synthetic(300) {
                s.add_document(d, None);
                let _ = s.execute("select.all", &json!({})).and_then(|_| s.execute(&format!("object.pathfinder.{op}"), &json!({})));
            }
        });
        rows.push((format!("Pathfinder {op}, 300 paths"), ms));
    }
    note("after Pathfinder");
    // Typing one character in the middle of a 20,000-character area-type story.
    let mut s = vectorcraft_engine::Session::new();
    s.add_document(Document::new(1600.0, 1200.0), None);
    let text: String = (0..2000).map(|i| if i % 12 == 11 { "paragraph.\n" } else { "lorem ips " }).collect();
    let id = s.execute("text.create", &json!({"x": 20, "y": 20, "text": text, "size": 10, "area": {"width": 1500, "height": 1100}})).map_err(err)?;
    let id = id.get("id").cloned().unwrap_or(json!(0));
    let mut at = 10_000;
    rows.push((
        "type one character, 20,000-character area text".into(),
        median_ms(9, || {
            at += 1;
            let _ = s.execute("text.editRange", &json!({"id": id, "start": at, "end": at, "insert": "x"}));
        }),
    ));
    note("after typing");
    // Four embedded 24 MP photos drawn at fit page by three renderers, as the app's canvas, render
    // worker and a panel each draw them.
    {
        let art = synthetic(3000)?;
        let mut d = Document::new(1600.0, 1200.0);
        let l = d.layers.first().map(|l| l.id);
        for i in 0..4u32 {
            let photo = Renderer::new().render(&art, 6000, 4000, Affine::scale(3.75 + f64::from(i) * 0.1), &opts).to_png()?;
            let key = format!("photo {i}");
            d.images.insert(key.clone(), vectorcraft_doc::ImageBlob::new("image/png", photo));
            let xf = Affine::translate((f64::from(i % 2) * 800.0, f64::from(i / 2) * 600.0)) * Affine::scale(800.0 / 6000.0);
            let image = vectorcraft_doc::ImageObject { key, width: 6000, height: 4000, xf, link: None, placement: Default::default(), pixels: None };
            let id = d.alloc_id();
            d.insert(l, usize::MAX, Node::new(id, vectorcraft_doc::NodeKind::Image(Box::new(image)))).map_err(|e| e.to_string())?;
        }
        note("4 embedded 24 MP photos");
        let mut renderers = [Renderer::new(), Renderer::new(), Renderer::new()];
        let ms = median_ms(1, || {
            for r in &mut renderers {
                r.render(&d, 2880, 1800, Affine::scale(1.6), &opts);
            }
        });
        rows.push(("render 4 embedded 24 MP photos with 3 renderers (cold)".into(), ms));
        note("after rendering the photos with 3 renderers");
    }
    // An SVG with one embedded picture opened from its text, as picture-heavy SVGs come: the
    // importer's passes over the text before usvg reads it, most of which is the data URL. The
    // picture is the synthetic page drawn at 1.25x (about 2 MB as a PNG; the row names its size);
    // the hidden rectangle is a marker, so the prepass walks the tree as it does for a file that
    // has one, instead of returning at its first check.
    {
        let k = 1.25;
        let (w, h) = ((SYNTHETIC_PAGE.0 * k).round() as u32, (SYNTHETIC_PAGE.1 * k).round() as u32);
        let photo = Renderer::new().render(&synthetic(3000)?, w, h, Affine::scale(k), &opts).to_png()?;
        let data = vectorcraft_svg::base64_encode(&photo);
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}"><image width="{w}" height="{h}" href="data:image/png;base64,{data}"/><rect width="100" height="100" style="display:none"/></svg>"#
        );
        // One import checked first, so a failure is reported rather than timed; the same text
        // imports the same way each time. Its document is kept for the memory reading.
        let imported = vectorcraft_svg::import_with_report(&svg).map_err(|e| e.to_string())?;
        let ms = median_ms(5, || drop(vectorcraft_svg::import_with_report(&svg)));
        rows.push((format!("import SVG, one embedded {} KB PNG", photo.len() / 1024), ms));
        note("after the import of an embedded picture");
        drop(imported);
    }
    // Output hashes, to compare builds: the SVG export and a save → open → save round trip.
    let fnv = |b: &[u8]| b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &x| (h ^ u64::from(x)).wrapping_mul(0x100_0000_01b3));
    let doc = synthetic(n)?;
    let svg = vectorcraft_svg::export(&doc, &vectorcraft_svg::ExportOptions::default());
    let saved = vectorcraft_format::save_file(&doc);
    let reopened = vectorcraft_format::load(&saved).map_err(|e| e.to_string())?;
    outln!("  output hash: SVG export {:016x}, save→open→save {:016x}", fnv(svg.as_bytes()), fnv(&vectorcraft_format::save_file(&reopened)));
    note("after the SVG export and save→open→save");
    outln!("  tracked (no budget yet)");
    for (name, ms) in &rows {
        outln!("{}", tracked_row(name, *ms));
    }
    // `rss_mb` reads the working set on Windows.
    let memory = if cfg!(windows) { "working set" } else { "memory" };
    for (what, mb, peak) in &mem {
        outln!("{}", memory_row(&format!("{memory} {what}"), *mb, *peak));
    }
    Ok(())
}

pub fn run(args: &[String]) -> Result<(), String> {
    if crate::help_requested(args, &["--paths"]) {
        return crate::help();
    }
    let mut n = 50_000usize;
    let mut want_deep = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--deep" => want_deep = true,
            "--paths" => n = it.next().and_then(|v| v.parse().ok()).ok_or("--paths needs a number")?,
            other => return Err(format!("unknown perf option `{other}`")),
        }
    }
    let cores = std::thread::available_parallelism().map_or(1, |c| c.get());
    // The figure is the 1-minute load average, or on Windows the cores busy over one second.
    #[cfg(not(windows))]
    let (load, label) = (load_average(), "load average");
    #[cfg(windows)]
    let (load, label) = (busy_cores(cores), "cores busy");
    outln!("VectorCraft performance budgets — {n} paths, {cores} cores, {label} {}", load.map_or("?".into(), |l| format!("{l:.1}")));
    let noisy = load.is_some_and(|l| l > cores as f64 * 0.75);
    if noisy {
        outln!("WARNING: the machine is busy; wall-clock timings below are not trustworthy.");
    }
    let t = Instant::now();
    let doc = synthetic(n)?;
    outln!("  (built in {:.0} ms)", t.elapsed().as_secs_f64() * 1000.0);

    let mut rows: Vec<(&str, f64, f64)> = vec![];
    let opts = RenderOptions::default();
    let mut r = Renderer::new();
    let fit = Affine::scale(1.6);
    r.render(&doc, 2880, 1800, fit, &opts);
    // The canvas never waits for these: while panning/zooming it shows the last frame reprojected
    // and re-renders in the background, so the budget is for that refresh to land quickly.
    rows.push(("render: fit page, 2880×1800", median_ms(5, || drop(r.render(&doc, 2880, 1800, fit, &opts))), 100.0));
    let mut dx = 0.0;
    rows.push((
        "render: background refresh after a pan",
        median_ms(9, || {
            dx += 7.0;
            drop(r.render(&doc, 2880, 1800, Affine::translate((dx, 0.0)) * fit, &opts));
        }),
        100.0,
    ));
    let zoom = Affine::scale(6.4) * Affine::translate((-600.0, -400.0));
    r.render(&doc, 2880, 1800, zoom, &opts);
    // Zoomed in, culling leaves few objects: this one should fit a frame.
    rows.push(("render: 400% zoom, 2880×1800", median_ms(5, || drop(r.render(&doc, 2880, 1800, zoom, &opts))), 16.0));

    let mut rng = Rng(7);
    let points: Vec<Point> = (0..200).map(|_| Point::new(rng.next() * 1600.0, rng.next() * 1200.0)).collect();
    let hits = median_ms(3, || {
        for p in &points {
            drop(hit::hit_test(&doc, *p, hit::HitOptions::default()));
        }
    }) / points.len() as f64;
    rows.push(("hit test (per click)", hits, 2.0));

    let mut bytes = vec![];
    rows.push(("save .vectorcraft", median_ms(3, || bytes = vectorcraft_format::save_file(&doc)), 300.0));
    let mut failed = None;
    let open = median_ms(3, || {
        if let Err(e) = vectorcraft_format::load(&bytes) {
            failed = Some(format!("open .vectorcraft: {e}"));
        }
    });
    rows.push(("open .vectorcraft", open, 300.0));
    let svg_opts = vectorcraft_svg::ExportOptions::default();
    rows.push(("export SVG", median_ms(3, || drop(vectorcraft_svg::export(&doc, &svg_opts))), 500.0));

    // Pathfinder Unite on 1,000 overlapping paths.
    let unite = median_ms(3, || {
        let mut s = vectorcraft_engine::Session::new();
        let r = synthetic(1000).and_then(|d| {
            s.add_document(d, None);
            s.execute("select.all", &json!({})).and_then(|_| s.execute("object.pathfinder.unite", &json!({}))).map_err(|e| e.to_string())
        });
        if let Err(e) = r {
            failed = Some(format!("Pathfinder Unite: {e}"));
        }
    });
    rows.push(("Pathfinder Unite, 1,000 paths", unite, 150.0));

    // Path editing: one pointer move of a Direct Selection drag (the tool event and the preview it
    // applies), on a 5,000-anchor path among the synthetic paths.
    let (handle, segment) = edit_drags(n).unwrap_or_else(|e| {
        failed = Some(e);
        (0.0, 0.0)
    });
    rows.push(("handle drag, 5,000-anchor path (per move)", handle, 8.0));
    rows.push(("segment drag, 5,000-anchor path (per move)", segment, 8.0));
    if let Some(e) = failed {
        return Err(e);
    }
    if want_deep {
        deep(n)?;
    }

    let mut over = 0;
    outln!("  {:<42} {:>10} {:>10}", "", "measured", "budget");
    for (name, ms, budget) in &rows {
        let ok = ms <= budget;
        over += usize::from(!ok);
        outln!("{}", budget_row(name, *ms, *budget, ok));
    }
    match (over, noisy) {
        (0, _) => Ok(()),
        (_, true) => {
            outln!("{over} over budget, but the machine is busy: re-run when idle.");
            Ok(())
        }
        _ => Err(format!("{over} budget(s) exceeded")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_share_leaves_out_idle_time() {
        // In one second four processors log 40,000,000 ticks of 100 ns of kernel and user time,
        // 10,000,000 of them idle.
        assert_eq!(busy_share([7, 100, 50], [10_000_007, 25_000_100, 15_000_050]), Some(0.75));
        // There is no share when no time passed, a time went back, or the idle time exceeds the
        // kernel and user time.
        assert_eq!(busy_share([7, 100, 50], [7, 100, 50]), None);
        assert_eq!(busy_share([7, 100, 50], [8, 99, 60]), None);
        assert_eq!(busy_share([0, 0, 0], [30, 10, 10]), None);
    }

    /// On Windows `busy_cores` gives a number between 0 and the cores.
    #[cfg(windows)]
    #[test]
    fn windows_reads_how_many_cores_are_busy() {
        let cores = std::thread::available_parallelism().map_or(1, |c| c.get());
        let busy = busy_cores(cores);
        assert!(busy.is_some_and(|b| (0.0..=cores as f64).contains(&b)), "{busy:?} of {cores}");
    }

    /// On Windows `rss_mb` gives this process's working set: more than 0 MB and less than
    /// 10,000 MB.
    #[cfg(windows)]
    #[test]
    fn windows_reads_the_working_set() {
        let mb = rss_mb().map(|(mb, _)| mb);
        assert!(mb.is_some_and(|mb| mb > 0.0 && mb < 10_000.0), "{mb:?}");
    }

    /// On Windows and Linux `rss_mb` also gives the peak: after this test writes 64 MB and frees
    /// it, the peak is no more than 16 MB below the resident memory read while it held the 64 MB.
    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn freed_memory_stays_in_the_peak() {
        let buffer = std::hint::black_box(vec![1u8; 64 << 20]);
        let held = rss_mb();
        drop(buffer);
        let (Some((held, _)), Some((now, peak))) = (held, rss_mb()) else {
            eprintln!("no memory reading (Linux without `ps`): nothing to check");
            return;
        };
        assert!(peak.is_some_and(|peak| peak > held - 16.0), "holding 64 MB: {held:.1} MB, after freeing it: {now:.1} MB, peak {peak:?}");
    }

    /// `vm_hwm_mb` reads the peak resident set size from the `VmHWM` line of a status text that
    /// also has a `VmRSS` line.
    #[test]
    fn vm_hwm_is_the_peak_resident_set_size() {
        let status = "Name:\tvectorcraft-cli\nVmPeak:\t 2097152 kB\nVmSize:\t 1572864 kB\nVmHWM:\t  819200 kB\nVmRSS:\t  204800 kB\n";
        assert_eq!(vm_hwm_mb(status), Some(800.0));
        // A text without a `VmHWM` line gives no peak.
        assert_eq!(vm_hwm_mb("Name:\tkthreadd\nState:\tS (sleeping)\n"), None);
    }

    /// A memory row ends with the peak when there is one; without a peak it ends with the value
    /// in MB. With the longest name on Windows, the value and the peak line up with the other rows.
    #[test]
    fn memory_rows_end_with_the_peak_when_there_is_one() {
        let row = memory_row("working set start", 346.4, Some(470.2));
        assert!(row.starts_with("  working set start ") && row.ends_with(" 346 MB  peak      470 MB"), "{row}");
        let longest = memory_row("working set after rendering the photos with 3 renderers", 2514.0, Some(2541.0));
        assert_eq!(longest.chars().count(), row.chars().count(), "{longest}\n{row}");
        let row = memory_row("memory start", 346.4, None);
        assert!(row.starts_with("  memory start ") && row.ends_with(" 346 MB") && !row.contains("peak"), "{row}");
    }

    /// With the longest names, the values of a tracked row and of the budget table's rows line
    /// up with the rows that have short names.
    #[test]
    fn the_longest_tracked_and_budget_names_fit_their_columns() {
        let longest = tracked_row("render 4 embedded 24 MP photos with 3 renderers (cold)", 774.61);
        let short = tracked_row("Pathfinder crop, 300 paths", 30.94);
        assert_eq!(longest.chars().count(), short.chars().count(), "{longest}\n{short}");
        for name in ["handle drag, 5,000-anchor path (per move)", "segment drag, 5,000-anchor path (per move)"] {
            let (longest, short) = (budget_row(name, 0.12, 8.0, true), budget_row("export SVG", 7.81, 500.0, true));
            assert_eq!(longest.chars().count(), short.chars().count(), "{longest}\n{short}");
        }
    }
}
