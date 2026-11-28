//! `cargo run --release -p vectorcraft-engine --example export_bench -- [FORMAT] [FILE…]` — wall
//! time of `document.export` (default SVG) for each file (default: the documents in `examples/`).
//! `ITERS` sets the runs per file (default 7); prints the median and the fastest run.
// Example (dev tool): unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::time::Instant;

use serde_json::json;
use vectorcraft_engine::Session;

fn main() {
    let mut args = std::env::args().skip(1);
    let format = args.next().unwrap_or_else(|| "svg".into());
    let mut files: Vec<PathBuf> = args.map(PathBuf::from).collect();
    if files.is_empty() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        files = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "vectorcraft")).collect();
        files.sort();
    }
    let iters: usize = std::env::var("ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(7).max(1);
    let out_dir = std::env::temp_dir().join(format!("vectorcraft-export-bench-{}", std::process::id()));
    std::fs::create_dir_all(&out_dir).unwrap();
    let mut total = 0.0;
    for file in &files {
        let mut s = Session::new();
        s.execute("document.open", &json!({ "path": file.to_string_lossy() })).unwrap();
        let name = file.file_stem().unwrap().to_string_lossy().into_owned();
        let out = out_dir.join(format!("{name}.{format}"));
        let p = json!({ "path": out.to_string_lossy(), "format": format });
        let mut ms: Vec<f64> = (0..iters)
            .map(|_| {
                let t = Instant::now();
                s.execute("document.export", &p).unwrap();
                t.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        ms.sort_by(f64::total_cmp);
        let median = ms[ms.len() / 2];
        total += median;
        let bytes = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
        println!("{name}.{format}: median {median:.1} ms, min {:.1} ms ({bytes} bytes)", ms[0]);
    }
    println!("total of medians: {total:.1} ms");
    std::fs::remove_dir_all(&out_dir).ok();
}
