//! Local Affinity documents (never committed): every file under `AFFINITY_SAMPLES` (searched
//! recursively) opens as editable art, and each artboard is compared with the thumbnail Affinity
//! stored in the file. Reports the mean difference per artboard (0–255, both on white); with
//! `AFFINITY_SAMPLES_OUT` set, writes each board's thumbnail crop and render side by side there.
//! Skipped when `AFFINITY_SAMPLES` is unset (as in CI).

use std::path::{Path, PathBuf};

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            files(&p, out);
        } else if p
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|x| ["af", "afdesign", "afpub", "afphoto"].contains(&x.to_ascii_lowercase().as_str()))
        {
            out.push(p);
        }
    }
}

fn on_white(p: &[u8]) -> [f64; 3] {
    let a = f64::from(p[3]) / 255.0;
    [0, 1, 2].map(|i| f64::from(p[i]) * a + 255.0 * (1.0 - a))
}

#[test]
fn local_affinity_documents_render_like_their_thumbnails() {
    let Some(root) = std::env::var_os("AFFINITY_SAMPLES").map(PathBuf::from) else { return };
    let out = std::env::var_os("AFFINITY_SAMPLES_OUT").map(PathBuf::from);
    let mut paths = Vec::new();
    files(&root, &mut paths);
    paths.sort();
    for path in paths {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let bytes = std::fs::read(&path).unwrap();
        let loaded = vectorcraft_engine::cmd::fileio::load(&name, &bytes).unwrap();
        assert!(!loaded.preview_only, "{name}: {:?}", loaded.warnings);
        let d = &loaded.doc;
        eprintln!("{name}: {} artboards, {} layers", d.artboards.len(), d.layers.len());
        for w in &loaded.warnings {
            eprintln!("  {w}");
        }
        let Ok(preview) = vectorcraft_affinity::preview(&bytes) else { continue };
        let thumb = image::load_from_memory(preview.png).unwrap().to_rgba8();
        let union = d.artboards.iter().map(|a| a.rect).reduce(|a, b| a.union(b)).unwrap();
        let scale = f64::from(thumb.width()) / union.width();
        let mut total = 0.0;
        for (i, board) in d.artboards.iter().enumerate() {
            let r = board.rect;
            let crop = |v: f64| (v * scale).round().max(0.0) as u32;
            let (x, y) = (crop(r.x0 - union.x0).min(thumb.width() - 1), crop(r.y0 - union.y0).min(thumb.height() - 1));
            let (w, h) = (crop(r.width()).min(thumb.width() - x).max(1), crop(r.height()).min(thumb.height() - y).max(1));
            let expected = image::imageops::crop_imm(&thumb, x, y, w, h).to_image();
            let k = (512.0 / r.width().max(r.height())).max(scale);
            let rendered = vectorcraft_render::Renderer::new().render_region(d, r, k, false);
            let big = image::RgbaImage::from_raw(rendered.width, rendered.height, rendered.to_straight()).unwrap();
            let got = image::imageops::resize(&big, w, h, image::imageops::FilterType::Triangle);
            let (mut sum, mut n) = (0.0, 0usize);
            for (p, q) in expected.as_raw().as_chunks::<4>().0.iter().zip(got.as_raw().as_chunks::<4>().0.iter()) {
                let (a, b) = (on_white(p), on_white(q));
                sum += (0..3).map(|c| (a[c] - b[c]).abs()).sum::<f64>();
                n += 3;
            }
            let diff = sum / n.max(1) as f64;
            total += diff;
            eprintln!("  board {i:3} {:<30} {diff:6.2}", board.name);
            if let Some(dir) = &out {
                let thumb_big = image::imageops::resize(&expected, big.width(), big.height(), image::imageops::FilterType::Triangle);
                let mut pair = image::RgbaImage::from_pixel(big.width() * 2 + 8, big.height(), image::Rgba([255, 255, 255, 255]));
                image::imageops::overlay(&mut pair, &thumb_big, 0, 0);
                image::imageops::overlay(&mut pair, &big, i64::from(big.width()) + 8, 0);
                let stem = path.file_stem().unwrap().to_string_lossy().to_string();
                std::fs::create_dir_all(dir).unwrap();
                pair.save(dir.join(format!("{stem}-{i:03}.png"))).unwrap();
            }
        }
        eprintln!("  mean over boards: {:.2}", total / d.artboards.len().max(1) as f64);
    }
}
