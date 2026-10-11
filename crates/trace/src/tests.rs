use super::*;
use vectorcraft_geom::Point;

const BLACK: [u8; 4] = [0, 0, 0, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

fn bw() -> TraceParams {
    TraceParams { ignore_white: true, ..TraceParams::default() }
}

fn disc(size: u32, cx: f64, cy: f64, r: f64) -> Raster {
    Raster::from_fn(size, size, |x, y| {
        let (dx, dy) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
        if dx * dx + dy * dy <= r * r { BLACK } else { WHITE }
    })
}

/// Net filled area (outer contours positive, holes negative).
fn net_area(p: &vectorcraft_geom::PathData) -> f64 {
    p.subpaths.iter().map(|s| s.area()).sum()
}

#[test]
fn black_disc_is_one_closed_path_with_disc_area() {
    let r = 80.0;
    let res = trace(&disc(200, 100.0, 100.0, r), &bw());
    assert_eq!(res.paths.len(), 1);
    let p = &res.paths[0];
    assert_eq!(p.path.subpaths.len(), 1);
    assert!(p.path.subpaths[0].closed);
    assert_eq!(p.color, [0, 0, 0]);
    let want = std::f64::consts::PI * r * r;
    let got = net_area(&p.path);
    assert!((got - want).abs() / want < 0.03, "area {got} vs {want}");
    // Curves, not a pixel staircase.
    assert!(p.path.anchor_count() < 40, "anchors {}", p.path.anchor_count());
}

#[test]
fn small_disc_area_within_tolerance() {
    let r = 20.0;
    let res = trace(&disc(64, 32.0, 32.0, r), &bw());
    assert_eq!(res.paths.len(), 1);
    let want = std::f64::consts::PI * r * r;
    let got = net_area(&res.paths[0].path);
    assert!((got - want).abs() / want < 0.03, "area {got} vs {want}");
}

#[test]
fn ring_has_outer_and_hole() {
    let img = Raster::from_fn(200, 200, |x, y| {
        let d2 = (x as f64 + 0.5 - 100.0).powi(2) + (y as f64 + 0.5 - 100.0).powi(2);
        if (40.0 * 40.0..=80.0 * 80.0).contains(&d2) { BLACK } else { WHITE }
    });
    let res = trace(&img, &bw());
    assert_eq!(res.paths.len(), 1);
    assert_eq!(res.paths[0].path.subpaths.len(), 2);
    let want = std::f64::consts::PI * (80.0f64.powi(2) - 40.0f64.powi(2));
    let got = net_area(&res.paths[0].path);
    assert!((got - want).abs() / want < 0.03, "area {got} vs {want}");
    // Opposite orientations → fills the same under non-zero.
    let a: Vec<f64> = res.paths[0].path.subpaths.iter().map(|s| s.area()).collect();
    assert!(a[0] * a[1] < 0.0);
}

#[test]
fn stripes_give_one_path_per_stripe() {
    // 5 black stripes, 10 px wide, separated by 10 px of white.
    let img = Raster::from_fn(100, 60, |x, _| if (x / 10) % 2 == 0 { BLACK } else { WHITE });
    let res = trace(&img, &bw());
    assert_eq!(res.paths.len(), 5);
    for p in &res.paths {
        let a = net_area(&p.path);
        assert!((a - 600.0).abs() < 1.0, "stripe area {a}");
        assert_eq!(p.path.anchor_count(), 4, "rectangles keep their 4 corners");
    }
}

#[test]
fn two_colour_stripes_without_ignore_white_trace_both_colours() {
    let img = Raster::from_fn(100, 60, |x, _| if (x / 10) % 2 == 0 { BLACK } else { WHITE });
    let res = trace(&img, &TraceParams::default());
    assert_eq!(res.paths.len(), 10);
    assert_eq!(res.palette.len(), 2);
    let total: f64 = res.paths.iter().map(|p| net_area(&p.path)).sum();
    assert!((total - 6000.0).abs() < 2.0);
}

#[test]
fn noise_removal_drops_speckles() {
    let img = Raster::from_fn(100, 100, |x, y| {
        let square = (20..60).contains(&x) && (20..60).contains(&y);
        let speck = (x % 13 == 3 && y % 11 == 5) && !(15..65).contains(&x);
        if square || speck { BLACK } else { WHITE }
    });
    let noisy = trace(&img, &TraceParams { noise: 0, ..bw() });
    assert!(noisy.paths.len() > 10, "{} paths", noisy.paths.len());
    let clean = trace(&img, &TraceParams { noise: 10, ..bw() });
    assert_eq!(clean.paths.len(), 1);
    assert!((net_area(&clean.paths[0].path) - 1600.0).abs() < 1.0);
}

#[test]
fn noise_removal_fills_pinholes() {
    let img = Raster::from_fn(60, 60, |x, y| {
        let square = (10..50).contains(&x) && (10..50).contains(&y);
        let hole = x == 30 && y == 30;
        if square && !hole { BLACK } else { WHITE }
    });
    let res = trace(&img, &TraceParams { noise: 4, ..bw() });
    assert_eq!(res.paths.len(), 1);
    assert_eq!(res.paths[0].path.subpaths.len(), 1);
}

#[test]
fn diagonal_pixels_stay_separate() {
    let img = Raster::from_fn(4, 4, |x, y| if (x, y) == (1, 1) || (x, y) == (2, 2) { BLACK } else { WHITE });
    let res = trace(&img, &TraceParams { noise: 0, ..bw() });
    assert_eq!(res.paths.len(), 2);
}

#[test]
fn contour_loops_are_consistent() {
    let mask = vec![true, true, true, true, false, true, true, true, true];
    let comps = trace_mask(&mask, 3, 3);
    assert_eq!(comps.len(), 1);
    assert_eq!(comps[0].pixels, 8);
    assert_eq!(comps[0].outer.area2, 18);
    assert_eq!(comps[0].holes.len(), 1);
    assert_eq!(comps[0].holes[0].area2, -2);
}

#[test]
fn color_mode_palette_size_matches_request() {
    let quad = |x: u32, y: u32| match (x < 50, y < 50) {
        (true, true) => [220, 30, 30, 255],
        (false, true) => [30, 200, 40, 255],
        (true, false) => [30, 40, 210, 255],
        (false, false) => [240, 220, 20, 255],
    };
    let img = Raster::from_fn(100, 100, quad);
    let p4 = trace(&img, &TraceParams { mode: Mode::Color, colors: 4, ..TraceParams::default() });
    assert_eq!(p4.palette.len(), 4);
    assert_eq!(p4.paths.len(), 4);
    let p2 = trace(&img, &TraceParams { mode: Mode::Color, colors: 2, ..TraceParams::default() });
    assert_eq!(p2.palette.len(), 2);
    // Palette colours are close to the originals.
    assert!(p4.palette.iter().any(|c| c[0] > 200 && c[1] < 60 && c[2] < 60));
}

#[test]
fn color_mode_gradient_uses_at_most_n_colors() {
    let img = Raster::from_fn(128, 32, |x, _| [(x * 2) as u8, 255 - (x * 2) as u8, 128, 255]);
    for n in [3, 6, 16] {
        let q = quantize(&img, &TraceParams { mode: Mode::Color, colors: n, ..TraceParams::default() });
        assert!(q.palette.len() <= n as usize && q.palette.len() >= 2, "n={n}: {}", q.palette.len());
    }
}

#[test]
fn grayscale_levels() {
    let img = Raster::from_fn(256, 8, |x, _| [x as u8, x as u8, x as u8, 255]);
    let q = quantize(&img, &TraceParams { mode: Mode::Grayscale, colors: 4, ..TraceParams::default() });
    assert_eq!(q.palette.len(), 4);
    assert!(q.palette.iter().all(|c| c[0] == c[1] && c[1] == c[2]));
    let res = trace(&img, &TraceParams { mode: Mode::Grayscale, colors: 4, noise: 0, ..TraceParams::default() });
    assert_eq!(res.paths.len(), 4);
}

#[test]
fn overlapping_layers_stack() {
    // White background with a black square: overlapping → bottom layer covers everything.
    let img = Raster::from_fn(50, 50, |x, y| if (10..30).contains(&x) && (10..30).contains(&y) { BLACK } else { WHITE });
    let ab = trace(&img, &TraceParams::default());
    let ov = trace(&img, &TraceParams { method: Method::Overlapping, ..TraceParams::default() });
    assert_eq!(ab.paths.len(), 2);
    assert_eq!(ov.paths.len(), 2);
    // Abutting: white has a hole; overlapping: white is a full rectangle underneath.
    assert_eq!(ab.paths[0].path.subpaths.len(), 2);
    assert_eq!(ov.paths[0].path.subpaths.len(), 1);
    assert!((net_area(&ov.paths[0].path) - 2500.0).abs() < 1.0);
    assert_eq!(ov.paths[1].color, [0, 0, 0]);
}

#[test]
fn transparent_pixels_are_not_traced() {
    let img = Raster::from_fn(40, 40, |x, _| if x < 20 { [0, 0, 0, 0] } else { BLACK });
    let res = trace(&img, &TraceParams::default());
    assert_eq!(res.paths.len(), 1);
    assert!((net_area(&res.paths[0].path) - 800.0).abs() < 1.0);
}

#[test]
fn threshold_controls_black_and_white() {
    let img = Raster::from_fn(40, 40, |x, _| if x < 20 { [100, 100, 100, 255] } else { WHITE });
    assert_eq!(trace(&img, &TraceParams { threshold: 128, ..bw() }).paths.len(), 1);
    assert_eq!(trace(&img, &TraceParams { threshold: 90, ..bw() }).paths.len(), 0);
}

#[test]
fn fidelity_changes_anchor_count() {
    let img = Raster::from_fn(200, 200, |x, y| {
        let t = (y as f64 / 12.0).sin() * 20.0 + 100.0;
        if (x as f64) < t { BLACK } else { WHITE }
    });
    let lo = trace(&img, &TraceParams { paths: 0.0, ..bw() }).anchor_count();
    let hi = trace(&img, &TraceParams { paths: 100.0, ..bw() }).anchor_count();
    assert!(hi >= lo, "hi {hi} lo {lo}");
}

#[test]
fn snap_curves_to_lines_straightens_near_lines() {
    // A slightly jagged, almost straight edge.
    let img = Raster::from_fn(200, 100, |x, y| if y as f64 > 30.0 + (x as f64 * 0.05) + if x % 37 < 2 { 1.0 } else { 0.0 } { BLACK } else { WHITE });
    let res = trace(&img, &TraceParams { snap_curves_to_lines: true, ..bw() });
    let sp = &res.paths[0].path.subpaths[0];
    let curves = (0..sp.segment_count()).filter(|&i| !sp.segment_is_line(i)).count();
    assert_eq!(curves, 0, "{sp:?}");
}

#[test]
fn presets_resolve() {
    assert_eq!(presets().len(), 13);
    for n in PRESET_NAMES {
        assert!(preset(n).is_some(), "{n}");
    }
    assert!(preset("[Default]").is_some());
    assert_eq!(preset("16 colors").unwrap().colors, 16);
    assert_eq!(preset("Shades of Gray").unwrap().mode, Mode::Grayscale);
    assert_eq!(preset("flat logo").unwrap().mode, Mode::Logo);
    assert!(preset("nope").is_none());
}

#[test]
fn params_json_roundtrip_and_partial() {
    let p = preset("6 Colors").unwrap();
    let v = serde_json::to_value(&p).unwrap();
    assert_eq!(v["mode"], "color");
    let back: TraceParams = serde_json::from_value(v).unwrap();
    assert_eq!(back, p);
    let partial: TraceParams = serde_json::from_value(serde_json::json!({"threshold": 90, "ignoreWhite": true})).unwrap();
    assert_eq!(partial.threshold, 90);
    assert!(partial.ignore_white);
    assert_eq!(partial.mode, Mode::BlackAndWhite);
}

#[test]
fn decode_png() {
    let img = disc(32, 16.0, 16.0, 10.0);
    let bytes = img.encode_png();
    let r = Raster::decode(&bytes).unwrap();
    assert_eq!(r, img);
    assert!(Raster::decode(b"not an image").is_err());
}

#[test]
fn all_presets_trace_a_colour_image() {
    let img = Raster::from_fn(64, 64, |x, y| [(x * 4) as u8, (y * 4) as u8, ((x + y) * 2) as u8, 255]);
    for (name, p) in presets() {
        // Flat Logo refuses a gradient on purpose (see logo::tests); every other preset traces it.
        if p.mode == Mode::Logo {
            continue;
        }
        let r = trace(&img, &p);
        assert!(!r.paths.is_empty() || p.ignore_white, "{name}");
        for tp in &r.paths {
            assert!(tp.path.subpaths.iter().all(|s| s.closed && s.anchors.iter().all(|a| a.p.x.is_finite())));
        }
    }
}

#[test]
#[ignore = "timing; run with --release --ignored"]
fn timing_1000x1000_bw_under_300ms() {
    let img = Raster::from_fn(1000, 1000, |x, y| {
        let (fx, fy) = (x as f64, y as f64);
        let v = (fx / 37.0).sin() * (fy / 23.0).cos() + ((fx + fy) / 91.0).sin() * 0.5;
        if v > 0.2 { BLACK } else { WHITE }
    });
    let t = std::time::Instant::now();
    let res = trace(&img, &TraceParams { ignore_white: true, ..TraceParams::default() });
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    eprintln!("1000x1000 B&W: {} paths, {} anchors in {ms:.1} ms", res.paths.len(), res.anchor_count());
    assert!(!res.paths.is_empty());
    assert!(ms < 300.0, "{ms} ms");
}

/// Stripes of `n` colours, each with a few dots of the next one: many layers, many small shapes.
fn stripes(size: u32, n: u32) -> Raster {
    let colour = |i: u32| [(i * 37 % 256) as u8, (i * 91 % 256) as u8, (i * 53 % 256) as u8, 255];
    Raster::from_fn(size, size, |x, y| {
        let band = x * n / size;
        if (x % 9 < 3) && (y % 9 < 3) { colour(band + 1) } else { colour(band) }
    })
}

/// #525: past the anchor budget the trace gives up with advice instead of making millions of
/// anchors; under it, it is the plain trace.
#[test]
fn a_trace_past_its_anchor_budget_is_refused() {
    let img = stripes(120, 8);
    let p = TraceParams { mode: Mode::Color, colors: 16, noise: 1, ..TraceParams::default() };
    let full = trace(&img, &p);
    let anchors = full.anchor_count();
    assert!(anchors > 100, "{anchors} anchors");
    assert!(matches!(trace_within(&img, &p, anchors / 2), Err(TraceError::TooComplex { max }) if max == anchors / 2));
    assert_eq!(trace_within(&img, &p, anchors).unwrap(), full);
}

/// Layers traced in parallel come out in the same order every time: bottom (largest) first.
#[test]
fn parallel_layers_keep_their_order() {
    let img = stripes(90, 12);
    let p = TraceParams { mode: Mode::Color, colors: 24, noise: 1, method: Method::Overlapping, ..TraceParams::default() };
    let a = trace(&img, &p);
    assert!(a.palette.len() > 8, "{} colours", a.palette.len());
    for _ in 0..4 {
        assert_eq!(trace(&img, &p), a);
    }
    // Overlapping: each layer's shapes cover the layers above, so the pixel counts fall upwards.
    let firsts: Vec<usize> = a.palette.iter().filter_map(|c| a.paths.iter().find(|t| t.color == *c).map(|t| t.pixels)).collect();
    assert!(firsts.windows(2).all(|w| w[0] >= w[1]), "{firsts:?}");
}

/// A PNG whose header says `w` × `h` pixels, with no image data.
fn png_header(w: u32, h: u32) -> Vec<u8> {
    fn crc(bytes: &[u8]) -> u32 {
        let mut c = !0u32;
        for &b in bytes {
            c ^= u32::from(b);
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    let mut ihdr = b"IHDR".to_vec();
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    for chunk in [ihdr, b"IDAT".to_vec(), b"IEND".to_vec()] {
        png.extend((chunk.len() as u32 - 4).to_be_bytes());
        png.extend(&chunk);
        png.extend(crc(&chunk).to_be_bytes());
    }
    png
}

/// An image too large to trace is refused from its header, before it is decoded.
#[test]
fn an_image_too_large_to_trace_is_refused_before_decoding() {
    let err = Raster::decode(&png_header(30_000, 30_000)).unwrap_err();
    assert!(matches!(err, TraceError::TooLarge { width: 30_000, height: 30_000, .. }), "{err}");
    assert!(matches!(Raster::decode(&png_header(64, 64)), Err(TraceError::Decode(_))), "a small header goes on to decoding");
}

/// Four flat colours with anti-aliased (blended) edges between the quadrants.
fn flat_quads() -> Raster {
    const C: [[u8; 3]; 4] = [[220, 30, 30], [30, 200, 40], [30, 40, 210], [240, 220, 20]];
    Raster::from_fn(100, 100, |x, y| {
        let at = |x: u32, y: u32| C[usize::from(x >= 50) + 2 * usize::from(y >= 50)];
        let (a, b) = (at(x, y), at(x.saturating_sub(1), y.saturating_sub(1)));
        // Blend the pixels along the quadrant edges, as an anti-aliased picture has them.
        let c: Vec<u8> = a.iter().zip(b).map(|(p, q)| ((u16::from(*p) + u16::from(q)) / 2) as u8).collect();
        [c[0], c[1], c[2], 255]
    })
}

/// A smooth two-dimensional colour gradient (continuous tone).
fn gradient() -> Raster {
    Raster::from_fn(128, 128, |x, y| [(x * 2) as u8, (y * 2) as u8, (255 - x - y / 2) as u8, 255])
}

fn palette(img: &Raster, palette: Palette, colors: u32, color_detail: f64) -> Vec<[u8; 3]> {
    let p = TraceParams { mode: Mode::Color, palette, colors, color_detail, noise: 4, ..TraceParams::default() };
    trace(img, &p).palette
}

#[test]
fn palettes_parse_from_their_ids_and_names() {
    for (id, want) in [
        ("limited", Palette::Limited),
        ("fullTone", Palette::FullTone),
        ("Automatic", Palette::Automatic),
        ("documentLibrary", Palette::DocumentLibrary),
        ("library", Palette::DocumentLibrary),
    ] {
        let p: TraceParams = serde_json::from_value(serde_json::json!({ "palette": id })).unwrap();
        assert_eq!(p.palette, want, "{id}");
    }
    assert_eq!(TraceParams::default().palette, Palette::Limited);
    // The library's colours are looked up at every trace, not saved with the settings.
    let v = serde_json::to_value(TraceParams { swatches: vec![[1, 2, 3]], ..TraceParams::default() }).unwrap();
    assert!(v.get("swatches").is_none() && v["library"] == "document" && v["colorDetail"] == 50.0, "{v}");
}

#[test]
fn full_tone_follows_the_tones_the_image_has() {
    // Flat art keeps its few colours (and the tones of its blended edges); a gradient gets many,
    // more with more detail.
    let flat = palette(&flat_quads(), Palette::FullTone, 6, 50.0);
    assert!((4..=10).contains(&flat.len()), "flat art: {flat:?}");
    let few = palette(&gradient(), Palette::FullTone, 6, 10.0).len();
    let many = palette(&gradient(), Palette::FullTone, 6, 90.0).len();
    assert!(few > 6 && many > few && many <= FULL_TONE_MAX, "{few} then {many} colours");
}

#[test]
fn automatic_traces_flat_art_with_its_colours_and_photos_in_full_tone() {
    let flat = palette(&flat_quads(), Palette::Automatic, 2, 50.0);
    assert_eq!(flat.len(), 4, "the four flat colours, whatever `colors` says: {flat:?}");
    assert!(flat.iter().any(|c| c[0] > 200 && c[1] < 60 && c[2] < 60), "red is one of them: {flat:?}");
    let tone = palette(&gradient(), Palette::Automatic, 2, 50.0);
    assert_eq!(tone.len(), palette(&gradient(), Palette::FullTone, 2, 50.0).len(), "continuous tone: as Full Tone");
}

#[test]
fn document_library_uses_the_most_used_swatches_exactly() {
    let swatches = vec![[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0], [0, 0, 0], [255, 255, 255]];
    let p = |colors| TraceParams {
        mode: Mode::Color,
        palette: Palette::DocumentLibrary,
        colors,
        swatches: swatches.clone(),
        noise: 4,
        ..TraceParams::default()
    };
    let hard = Raster::from_fn(100, 100, |x, y| match (x < 50, y < 50) {
        (true, true) => [220, 30, 30, 255],
        (false, true) => [30, 200, 40, 255],
        (true, false) => [30, 40, 210, 255],
        (false, false) => [240, 220, 20, 255],
    });
    let res = trace(&hard, &p(6));
    let mut got = res.palette.clone();
    got.sort_unstable();
    assert_eq!(got, [[0, 0, 255], [0, 255, 0], [255, 0, 0], [255, 255, 0]], "the nearest swatch of each colour, as it is");
    assert!(res.paths.iter().all(|t| swatches.contains(&t.color)));
    // At most `colors` of them: the ones the most pixels are nearest to.
    let two = trace(&hard, &p(2)).palette;
    assert!(two.len() <= 2 && two.iter().all(|c| swatches.contains(c)), "{two:?}");
    // Without library colours, it traces as Limited.
    let none = TraceParams { swatches: vec![], ..p(4) };
    assert_eq!(trace(&hard, &none).palette.len(), 4);
}

fn strokes(fills: bool) -> TraceParams {
    TraceParams { strokes: true, fills, stroke_width: 6.0, noise: 4, ..bw() }
}

/// Endpoints (first and last anchor) of each open subpath of the stroked paths.
fn stroked(res: &TraceResult) -> Vec<(&TracedPath, &vectorcraft_geom::SubPath)> {
    res.paths.iter().filter(|p| p.stroke.is_some()).flat_map(|p| p.path.subpaths.iter().map(move |s| (p, s))).collect()
}

#[test]
fn a_thin_line_traces_as_one_stroked_centre_line() {
    // A 3 px wide diagonal line from (20, 20) to (180, 100).
    let img = Raster::from_fn(200, 120, |x, y| {
        let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
        let t = (((px - 20.0) * 160.0 + (py - 20.0) * 80.0) / (160.0f64.powi(2) + 80.0f64.powi(2))).clamp(0.0, 1.0);
        let (cx, cy) = (20.0 + 160.0 * t, 20.0 + 80.0 * t);
        if (px - cx).hypot(py - cy) <= 1.5 { BLACK } else { WHITE }
    });
    let res = trace(&img, &strokes(true));
    assert_eq!(res.paths.len(), 1, "{:?}", res.paths.iter().map(|p| p.stroke).collect::<Vec<_>>());
    let p = &res.paths[0];
    let width = p.stroke.expect("stroked");
    assert!((2.0..=4.0).contains(&width), "width {width}");
    assert_eq!(p.path.subpaths.len(), 1);
    let sp = &p.path.subpaths[0];
    assert!(!sp.closed);
    let (a, b) = (sp.anchors[0].p, sp.anchors[sp.anchors.len() - 1].p);
    let (a, b) = if a.x < b.x { (a, b) } else { (b, a) };
    assert!(a.distance(Point::new(20.0, 20.0)) < 3.0 && b.distance(Point::new(180.0, 100.0)) < 3.0, "{a:?} … {b:?}");
    // A straight line: a few anchors, all on it.
    assert!(sp.anchors.len() <= 4, "{} anchors", sp.anchors.len());
    // Without Create Strokes, the same line is a filled outline.
    let filled = trace(&img, &bw());
    assert!(filled.paths.iter().all(|p| p.stroke.is_none()) && filled.paths[0].path.subpaths[0].closed);
}

#[test]
fn a_thin_ring_traces_as_a_closed_centre_line() {
    let img = Raster::from_fn(120, 120, |x, y| {
        let d = (x as f64 + 0.5 - 60.0).hypot(y as f64 + 0.5 - 60.0);
        if (d - 40.0).abs() <= 1.5 { BLACK } else { WHITE }
    });
    let res = trace(&img, &strokes(true));
    let lines = stroked(&res);
    assert_eq!(lines.len(), 1, "one ring");
    let sp = lines[0].1;
    assert!(sp.closed);
    let len = PathData::single(sp.clone()).length();
    let want = std::f64::consts::TAU * 40.0;
    assert!((len - want).abs() / want < 0.05, "length {len} vs {want}");
}

#[test]
fn wide_areas_stay_filled_and_lines_cross_at_junctions() {
    // A disc (r = 20) and, apart from it, a plus sign of 3 px wide bars.
    let img = Raster::from_fn(200, 100, |x, y| {
        let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
        let disc = (px - 50.0).hypot(py - 50.0) <= 20.0;
        let plus = ((px - 150.0).abs() <= 1.5 && (20.0..=80.0).contains(&py)) || ((py - 50.0).abs() <= 1.5 && (120.0..=180.0).contains(&px));
        if disc || plus { BLACK } else { WHITE }
    });
    let res = trace(&img, &strokes(true));
    let fills: Vec<_> = res.paths.iter().filter(|p| p.stroke.is_none()).collect();
    assert_eq!(fills.len(), 1, "the disc");
    let a = net_area(&fills[0].path);
    assert!((a - std::f64::consts::PI * 400.0).abs() < 60.0, "disc area {a}");
    let lines = stroked(&res);
    assert!((2..=4).contains(&lines.len()), "{} centre lines", lines.len());
    // The centre lines stop about half a width short of each end of the bars (their round caps
    // cover it).
    let total: f64 = lines.iter().map(|(_, s)| PathData::single((*s).clone()).length()).sum();
    assert!((106.0..=120.0).contains(&total), "both bars' length: {total}");
    let ends: Vec<Point> = lines.iter().flat_map(|(_, s)| [s.anchors[0].p, s.anchors[s.anchors.len() - 1].p]).collect();
    for want in [Point::new(150.0, 20.0), Point::new(150.0, 80.0), Point::new(120.0, 50.0), Point::new(180.0, 50.0)] {
        assert!(ends.iter().any(|e| e.distance(want) < 4.0), "a line ends near {want:?}: {ends:?}");
    }
    // Strokes only: the disc is outlined.
    let res = trace(&img, &strokes(false));
    assert!(res.paths.iter().all(|p| p.stroke.is_some()));
    assert!(res.paths.iter().any(|p| p.stroke == Some(1.0) && p.path.subpaths[0].closed), "the disc's outline");
}

#[test]
fn create_settings_parse_with_defaults() {
    let p: TraceParams = serde_json::from_value(serde_json::json!({ "strokes": true, "strokeWidth": 4 })).unwrap();
    assert!(p.fills && p.strokes && p.stroke_width == 4.0);
    let d = TraceParams::default();
    assert!(d.fills && !d.strokes && d.stroke_width == 10.0);
}

#[test]
fn create_strokes_survives_noise_and_tiny_images() {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let noise: Vec<u8> = (0..120 * 120).flat_map(|_| if next() % 3 == 0 { BLACK } else { WHITE }).collect();
    let noise = Raster::new(120, 120, noise);
    for img in [noise, Raster::from_fn(1, 1, |_, _| BLACK), Raster::from_fn(3, 2, |x, _| if x == 1 { BLACK } else { WHITE })] {
        for (fills, width) in [(true, 1.0), (false, 4.0), (true, 100.0)] {
            for noise in [0, 1, 4] {
                let p = TraceParams { strokes: true, fills, stroke_width: width, noise, ..TraceParams::default() };
                for t in trace(&img, &p).paths {
                    assert!(t.stroke.is_none_or(|w| w.is_finite() && w >= 1.0), "{:?}", t.stroke);
                }
            }
        }
    }
}

/// The 4-connected same-label components of `labels` (transparent pixels left out): each one's size
/// and whether it touches a pixel of another opaque label.
fn components(labels: &[u16], w: usize, h: usize) -> Vec<(usize, bool)> {
    let mut seen = vec![false; w * h];
    let mut out = vec![];
    for start in 0..w * h {
        if seen[start] || labels[start] == TRANSPARENT {
            continue;
        }
        let l = labels[start];
        let (mut size, mut touches, mut stack) = (0, false, vec![start]);
        seen[start] = true;
        while let Some(i) = stack.pop() {
            size += 1;
            let (x, y) = (i % w, i / w);
            let near = [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)];
            for j in near.into_iter().flatten() {
                if labels[j] == l {
                    if !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                } else if labels[j] != TRANSPARENT {
                    touches = true;
                }
            }
        }
        out.push((size, touches));
    }
    out
}

/// #819: a speck merged into a small region that is itself merged later was left behind as a new
/// island of the small region's old label.
#[test]
fn denoise_leaves_no_island_behind_a_merged_region() {
    // One row: a 1-pixel speck, a 2-pixel region, then a large one; nothing under 3 pixels may stay.
    let mut labels = vec![0, 1, 1, 2, 2, 2, 2, 2];
    denoise(&mut labels, 8, 1, 3);
    assert_eq!(labels, vec![2; 8]);
}

/// On noise, Remove Noise leaves no region under the minimum that could be merged, and a larger
/// minimum never leaves more regions (#819: more noise removal gave more paths).
#[test]
fn denoise_merges_every_small_region_and_more_noise_never_means_more_regions() {
    let (w, h) = (48, 40);
    let mut seed = 0x2545_f491_u32;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    for colors in [2u16, 3, 5, 8] {
        // Blocky noise: random 1-4 pixel runs, so regions of every size touch each other.
        let mut base = vec![0u16; w * h];
        let mut i = 0;
        while i < w * h {
            let (l, run) = ((next() % u32::from(colors)) as u16, 1 + next() as usize % 4);
            for p in base.iter_mut().skip(i).take(run) {
                *p = l;
            }
            i += run;
        }
        let mut last = usize::MAX;
        for min_area in [1usize, 2, 4, 8, 16, 32, 64, 128] {
            let mut labels = base.clone();
            denoise(&mut labels, w, h, min_area);
            let comps = components(&labels, w, h);
            if min_area > 1 {
                assert!(
                    comps.iter().all(|&(size, touches)| size >= min_area || !touches),
                    "{colors} colours, min {min_area}: a region under the minimum is left: {comps:?}"
                );
            }
            assert!(comps.len() <= last, "{colors} colours: min {min_area} leaves {} regions, more than the {last} a smaller one left", comps.len());
            last = comps.len();
        }
    }
}

/// #819: on noisy scan-like images, raising Noise never gives more paths. Before the fix, these
/// two went from 70 paths to 126 to 206 as Noise rose from 50 to 200 (and 22 to 25).
#[test]
fn more_noise_removal_never_gives_more_paths() {
    for (seed0, colors) in [(15u32, 2usize), (9, 6)] {
        let mut seed = seed0.wrapping_mul(0x9e37_79b9) | 1;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let ink: Vec<[u8; 4]> = (0..colors)
            .map(|_| {
                let v = next();
                [(v & 255) as u8, (v >> 8 & 255) as u8, (v >> 16 & 255) as u8, 255]
            })
            .collect();
        // Diagonal bands of ink with a quarter of the pixels speckled at random.
        let (w, h) = (121, 101);
        let px: Vec<[u8; 4]> = (0..w * h)
            .map(|i| {
                let band = ((i % w) / 7 + (i / w) / 5) % colors;
                if next() % 4 == 0 { ink[next() as usize % colors] } else { ink[band] }
            })
            .collect();
        let img = Raster::from_fn(120, 100, |x, y| px[y as usize * w + x as usize]);
        let mut last = (0, usize::MAX);
        for noise in [1, 4, 8, 12, 25, 50, 100, 200] {
            let n = trace(&img, &TraceParams { noise, ..TraceParams::default() }).paths.len();
            assert!(n <= last.1, "image {seed0}: Noise {noise} gives {n} paths, more than the {} of Noise {}", last.1, last.0);
            last = (noise, n);
        }
    }
}
