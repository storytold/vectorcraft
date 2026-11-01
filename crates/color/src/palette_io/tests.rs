use super::*;
use crate::{Gradient, GradientPaint, GradientStop};
use proptest::prelude::*;

#[test]
fn gpl_blank_name_keeps_fallback() {
    let blank = "GIMP Palette\nName: \nColumns: 0\n#\n  1   2   3\tx\n";
    assert_eq!(read(blank, "fallback").unwrap().name, "fallback");
    let named = "GIMP Palette\nName: Warm\n  1   2   3\tx\n";
    assert_eq!(read(named, "fallback").unwrap().name, "Warm");
}

/// A color book's string: a `u32` count of UTF-16 code units, then the units.
fn acb_str(out: &mut Vec<u8>, s: &str) {
    let units: Vec<u16> = s.encode_utf16().collect();
    out.extend((units.len() as u32).to_be_bytes());
    units.iter().for_each(|u| out.extend(u.to_be_bytes()));
}

/// A color book in `space` (0 RGB, 2 CMYK, 7 Lab) with `colors` (name, components) and an
/// optional spot/process marker.
fn acb(space: u16, colors: &[(&str, &[u8])], marker: &[u8]) -> Vec<u8> {
    let mut b = b"8BCB".to_vec();
    b.extend(1u16.to_be_bytes());
    b.extend(3000u16.to_be_bytes());
    for s in ["$$$/colorbook/Test/title=Test Book^R", "TEST ", " C", "a book"] {
        acb_str(&mut b, s);
    }
    b.extend((colors.len() as u16).to_be_bytes());
    b.extend(7u16.to_be_bytes());
    b.extend(0u16.to_be_bytes());
    b.extend(space.to_be_bytes());
    for (name, comps) in colors {
        acb_str(&mut b, name);
        b.extend(b"TST001");
        b.extend(*comps);
    }
    b.extend(marker);
    b
}

/// #831: color books (`.acb`) are read: their colors in RGB, CMYK or Lab, named with the book's
/// prefix and suffix, spot unless the book says process; page padding is left out.
#[test]
fn color_books_are_read() {
    let lib = read_bytes(&acb(0, &[("185", &[228, 0, 43]), ("", &[0, 0, 0]), ("Red 032", &[239, 51, 64])], b"spflspot"), "fallback").unwrap();
    assert_eq!(lib.name, "Test Book®");
    let names: Vec<&str> = lib.swatches.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["TEST 185 C", "TEST Red 032 C"]);
    assert!(lib.swatches.iter().all(|s| s.spot && s.global));
    assert_eq!(lib.swatches[0].paint.color(), Some(Color::rgb8(228, 0, 43)));
    // CMYK ink is stored inverted; a book marked process gives process colors.
    let lib = read_bytes(&acb(2, &[("Cyan", &[0, 255, 255, 255])], b"spflproc"), "x").unwrap();
    assert_eq!(lib.swatches[0].paint.color(), Some(Color::cmyk(1.0, 0.0, 0.0, 0.0)));
    assert!(!lib.swatches[0].spot && !lib.swatches[0].global);
    // Lab: lightness 0..255 for 0..100, a and b offset by 128; no marker: spot.
    let lib = read_bytes(&acb(7, &[("Grey", &[255, 128, 128])], b""), "x").unwrap();
    assert_eq!(lib.swatches[0].paint.color(), Some(Color::lab(100.0, 0.0, 0.0)));
    assert!(lib.swatches[0].spot);
    assert!(sniff_bytes(&acb(0, &[], b"")));
    // Unsupported versions and color spaces, and short files, are errors.
    let mut v2 = acb(0, &[], b"");
    v2[5] = 2;
    assert!(read_bytes(&v2, "x").is_err());
    assert!(read_bytes(&acb(1, &[("x", &[1, 2, 3])], b""), "x").is_err());
    let full = acb(0, &[("185", &[228, 0, 43])], b"");
    for cut in 4..full.len() {
        assert!(read_bytes(&full[..cut], "x").is_err(), "{cut} bytes");
    }
    let mut huge = b"8BCB\0\x01\0\x01".to_vec();
    huge.extend(u32::MAX.to_be_bytes());
    assert!(read_bytes(&huge, "x").is_err(), "a string longer than the file");
}

fn sample() -> SwatchLibrary {
    let grad = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(
        GradientKind::Linear,
        vec![
            GradientStop { midpoint: 0.4, ..GradientStop::new(0.0, Color::cmyk(0.1, 0.2, 0.3, 0.0)) },
            GradientStop { opacity: 0.5, ..GradientStop::new(1.0, Color::rgb8(255, 0, 0)) },
        ],
    ))));
    SwatchLibrary {
        name: "Brand */ Colours".into(),
        swatches: vec![
            Swatch { name: "Ink".into(), paint: Paint::solid(Color::cmyk(1.0, 0.5, 0.0, 0.2)), global: true, spot: true },
            Swatch { name: "Fade".into(), paint: grad, global: false, spot: false },
            Swatch { name: "Tiles".into(), paint: Paint::Pattern { pattern: "Dots".into(), xf: Default::default() }, global: false, spot: false },
        ],
        groups: vec![SwatchGroup {
            name: "Neutrals".into(),
            swatches: vec![
                Swatch { name: "R=255 G=0 B=0".into(), paint: Paint::solid(Color::rgb8(255, 0, 0)), global: false, spot: false },
                Swatch { name: "Mist".into(), paint: Paint::solid(Color::gray(0.25)), global: true, spot: false },
            ],
        }],
    }
}

/// `lib` written as text format `f`.
fn written(lib: &SwatchLibrary, f: PaletteFormat) -> String {
    String::from_utf8(write(lib, f).unwrap()).unwrap()
}

#[test]
fn native_round_trip_keeps_cmyk_spot_and_groups() {
    let lib = sample();
    let text = written(&lib, PaletteFormat::Native);
    assert!(sniff(&text));
    let back = read(&text, "fallback").unwrap();
    let mut expected = lib.clone();
    expected.swatches.retain(|w| !matches!(w.paint, Paint::Pattern { .. }));
    assert_eq!(back, expected, "everything but the pattern comes back exactly");
    assert!(back.swatches[0].spot && back.swatches[0].global);
    assert_eq!(back.swatches[0].paint.color(), Some(Color::cmyk(1.0, 0.5, 0.0, 0.2)));
    assert!(read("{\"format\": \"other\", \"version\": 1, \"name\": \"x\"}", "x").is_err());
}

#[test]
fn gpl_writes_rgb_with_group_headers_and_reads_comments_and_headers() {
    let text = written(&sample(), PaletteFormat::Gpl);
    assert!(text.starts_with("GIMP Palette\nName: Brand */ Colours\n"));
    assert!(text.contains("# Group: Neutrals\n255   0   0\tR=255 G=0 B=0\n"), "{text}");
    assert!(!text.contains("Fade") && !text.contains("Tiles"), "only solid colours");
    let back = read(&text, "x").unwrap();
    assert_eq!(back.name, "Brand */ Colours");
    assert_eq!(back.swatches.len(), 1);
    assert_eq!(back.groups[0].swatches[1].paint.color(), Some(Color::rgb8(191, 191, 191)), "grey as 8-bit RGB");
    // Another tool's palette: comments, a Columns header, blank lines, unnamed and repeated names.
    let other = "GIMP Palette\r\nName: Sunset\r\nColumns: 4\r\n# a comment\r\n\r\n  0 128 255\tSky Blue\r\n10 20 30\r\n0 0 0 Black\r\n1 1 1 Black\r\nbad line\r\n";
    let lib = read(other, "file").unwrap();
    assert_eq!(lib.name, "Sunset");
    let names: Vec<&str> = lib.swatches.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(names, ["Sky Blue", "R=10 G=20 B=30", "Black", "Black 2"]);
    assert_eq!(lib.swatches[0].paint.color(), Some(Color::rgb8(0, 128, 255)));
    assert_eq!(read("GIMP Palette\n1 2 3 x\n", "Fallback").unwrap().name, "Fallback");
    // Colors and groups share names; an unnamed group is "Color Group".
    let lib = read("GIMP Palette\n0 0 0 Black\n# Group: Black\n1 1 1 Black\n# Group:\n", "x").unwrap();
    let groups: Vec<(&str, Vec<&str>)> = lib.groups.iter().map(|g| (g.name.as_str(), g.swatches.iter().map(|w| w.name.as_str()).collect())).collect();
    assert_eq!(groups, [("Black 2", vec!["Black 3"]), ("Color Group", vec![])]);
}

#[test]
fn css_escapes_names_and_writes_gradients() {
    assert_eq!(css_property("Sky Blue"), "--sky-blue");
    assert_eq!(css_property("R=255 G=0 B=0"), "--r\\=255-g\\=0-b\\=0");
    assert_eq!(css_property("#FF00CC"), "--\\#ff00cc");
    assert_eq!(css_property("Café 50%"), "--café-50\\%");
    assert_eq!(css_property("  "), "--swatch");
    let text = written(&sample(), PaletteFormat::Css);
    assert!(text.starts_with("/* Brand * / Colours */\n:root {\n"), "{text}");
    assert!(text.contains("  --ink: #"), "{text}");
    assert!(text.contains("; /* spot */"));
    assert!(text.contains("  --fade: linear-gradient(90deg, #"), "{text}");
    assert!(text.contains("#ff000080 100%)"), "half-transparent stop: {text}");
    assert!(text.contains("  /* Neutrals */\n  --r\\=255-g\\=0-b\\=0: #ff0000;\n"));
    assert!(!text.contains("tiles"));
    // Names that collide once escaped are numbered.
    let lib = SwatchLibrary {
        name: "x".into(),
        swatches: ["Red", "red"].map(|n| Swatch { name: n.into(), paint: Paint::solid(Color::BLACK), global: false, spot: false }).to_vec(),
        groups: vec![],
    };
    let text = written(&lib, PaletteFormat::Css);
    assert!(text.contains("--red: #000000;") && text.contains("--red-2: #000000;"));
    assert!(read(&text, "x").is_err(), "CSS is written only");
}

#[test]
fn css_preserves_fractional_gradient_stop_positions() {
    for kind in [GradientKind::Linear, GradientKind::Radial] {
        let lib = SwatchLibrary {
            name: "Positions".into(),
            swatches: vec![Swatch {
                name: "Ramp".into(),
                paint: Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(
                    kind,
                    vec![
                        GradientStop::new(0.0, Color::BLACK),
                        GradientStop::new(0.125, Color::rgb8(255, 0, 0)),
                        GradientStop::new(0.12890625, Color::WHITE),
                        GradientStop::new(1.0, Color::rgb8(0, 0, 255)),
                    ],
                )))),
                global: false,
                spot: false,
            }],
            groups: vec![],
        };
        let text = written(&lib, PaletteFormat::Css);
        assert!(text.contains("#000000 0%, #ff0000 12.5%, #ffffff 12.890625%, #0000ff 100%"), "{kind:?}: {text}");
    }
}

#[test]
fn holds_counts_the_swatches_each_format_writes() {
    // `sample()` holds the spot color Ink, the linear gradient Fade, the pattern Tiles and two solid colors in a group.
    let mut lib = sample();
    let Paint::Gradient(mut mesh) = lib.swatches[1].paint.clone() else { panic!("Fade is a gradient") };
    mesh.gradient.kind = GradientKind::Freeform;
    lib.swatches.push(Swatch { name: "Mesh".into(), paint: Paint::Gradient(mesh), global: false, spot: false });
    lib.swatches.push(Swatch { name: "[None]".into(), paint: Paint::None, global: false, spot: false });
    let held = |f: PaletteFormat| lib.iter().filter(|w| f.holds(&w.paint)).count();
    assert_eq!(PaletteFormat::ALL.map(held), [5, 3, 3, 4], "vcswatches, gpl, ase, css");
    for f in [PaletteFormat::Native, PaletteFormat::Gpl, PaletteFormat::Ase] {
        assert_eq!(read_bytes(&write(&lib, f).unwrap(), "x").unwrap().len(), held(f), "{f:?}");
    }
    let css = written(&lib, PaletteFormat::Css);
    assert_eq!(css.lines().filter(|l| l.starts_with("  --")).count(), held(PaletteFormat::Css), "one property each: {css}");
}

/// A swatch exchange file of `blocks` (type, body).
fn ase(blocks: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut out = b"ASEF\0\x01\0\0".to_vec();
    out.extend((blocks.len() as u32).to_be_bytes());
    for (kind, body) in blocks {
        out.extend(kind.to_be_bytes());
        out.extend((body.len() as u32).to_be_bytes());
        out.extend(body);
    }
    out
}

/// A name as `.ase` stores it.
fn ase_name(name: &str) -> Vec<u8> {
    let units: Vec<u16> = name.encode_utf16().chain([0]).collect();
    let mut out = (units.len() as u16).to_be_bytes().to_vec();
    out.extend(units.iter().flat_map(|u| u.to_be_bytes()));
    out
}

/// A color block: name, model, components and color type.
fn ase_color(name: &str, model: &[u8; 4], values: &[f32], kind: u16) -> (u16, Vec<u8>) {
    let mut body = ase_name(name);
    body.extend(model);
    body.extend(values.iter().flat_map(|v| v.to_be_bytes()));
    body.extend(kind.to_be_bytes());
    (ASE_COLOR, body)
}

#[test]
fn ase_reads_color_models_kinds_and_groups() {
    let bytes = ase(&[
        ase_color("Sky", b"RGB ", &[0.0, 0.5, 1.0], 0),
        ase_color("Ink", b"CMYK", &[1.0, 0.5, 0.0, 0.2], 1),
        (ASE_GROUP_START, ase_name("Neutrals")),
        ase_color("Mist", b"Gray", &[0.75], 2),
        ase_color("Clay", b"LAB ", &[0.5, 20.0, -30.0], 2),
        (ASE_GROUP_END, vec![]),
        ase_color("Paper", b"RGB ", &[1.0, 1.0, 1.0], 2),
    ]);
    assert!(sniff_bytes(&bytes));
    let sw = |name: &str, color, global, spot| Swatch { name: name.into(), paint: Paint::solid(color), global, spot };
    let expected = SwatchLibrary {
        name: "Brand".into(),
        swatches: vec![
            sw("Sky", Color::rgb(0.0, 0.5, 1.0), true, false),
            sw("Ink", Color::cmyk(1.0, 0.5, 0.0, 0.2), true, true),
            sw("Paper", Color::WHITE, false, false),
        ],
        groups: vec![SwatchGroup {
            name: "Neutrals".into(),
            swatches: vec![sw("Mist", Color::gray(0.25), false, false), sw("Clay", Color::lab(50.0, 20.0, -30.0), false, false)],
        }],
    };
    assert_eq!(read_bytes(&bytes, "Brand").unwrap(), expected, "named after the file; 0 global, 1 spot, 2 process");
    // A lightness above 1 is L* itself.
    let deep = read_bytes(&ase(&[ase_color("Deep", b"LAB ", &[53.0, 20.0, -30.0], 2)]), "x").unwrap();
    assert_eq!(deep.swatches[0].paint.color(), Some(Color::lab(53.0, 20.0, -30.0)));
    // Text libraries read through the same call, also when they aren't valid UTF-8.
    assert!(sniff_bytes(b"GIMP Palette\n1 2 3 x\n") && !sniff_bytes(b"ASE") && !sniff_bytes(&[0xff, 0xfe, 0]));
    assert_eq!(read_bytes(b"GIMP Palette\n1 2 3 x\n", "F").unwrap().swatches.len(), 1);
    let latin1 = b"GIMP Palette\n1 2 3 Caf\xe9\n";
    assert!(sniff_bytes(latin1));
    assert_eq!(read_bytes(latin1, "F").unwrap().swatches.len(), 1);
}

#[test]
fn ase_names_unnamed_and_repeated_entries_and_skips_other_blocks() {
    let mut untyped = ase_color("Plain", b"RGB ", &[0.2, 0.4, 0.6], 2);
    untyped.1.truncate(untyped.1.len() - 2);
    let bytes = ase(&[
        ase_color("Café 🎨", b"RGB ", &[1.0, 0.0, 0.0], 2),
        ase_color("", b"CMYK", &[0.1, 0.2, 0.3, 0.0], 2),
        ase_color(" Red ", b"RGB ", &[2.0, -1.0, f32::NAN], 0),
        ase_color("Red", b"Gray", &[0.0], 2),
        untyped,
        ase_color("Hue", b"HSB ", &[0.1, 0.2, 0.3], 2),
        (0x0042, vec![1, 2, 3]),
        (ASE_GROUP_END, vec![]),
        (ASE_GROUP_START, ase_name("")),
        (ASE_GROUP_START, ase_name("Red")),
        ase_color("Red", b"Gray", &[1.0], 2),
    ]);
    let lib = read_bytes(&bytes, "x").unwrap();
    let names: Vec<&str> = lib.swatches.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(names, ["Café 🎨", "C=10 M=20 Y=30 K=0", "Red", "Red 2", "Plain"], "an HSB color and an unknown block are skipped");
    assert_eq!(lib.swatches[2].paint.color(), Some(Color::rgb(1.0, 0.0, 0.0)), "clamped, NaN read as 0");
    assert!(lib.swatches[2].global && !lib.swatches[4].global, "without a color type: process");
    // A group start ends the open group, and groups and colors share names.
    let groups: Vec<(&str, Vec<&str>)> = lib.groups.iter().map(|g| (g.name.as_str(), g.swatches.iter().map(|w| w.name.as_str()).collect())).collect();
    assert_eq!(groups, [("Color Group", vec![]), ("Red 3", vec!["Red 4"])]);
    assert_eq!(lib.groups[1].swatches[0].paint.color(), Some(Color::gray(0.0)), "gray level 1 is white");
}

#[test]
fn damaged_ase_files_are_errors() {
    let good = ase(&[(ASE_GROUP_START, ase_name("G")), ase_color("Sky", b"RGB ", &[0.0, 0.5, 1.0], 0)]);
    assert_eq!(read_bytes(&good, "x").unwrap().len(), 1);
    for n in ASE_SIGNATURE.len()..good.len() {
        assert!(read_bytes(&good[..n], "x").is_err(), "cut at {n}");
    }
    let mut v2 = good.clone();
    v2[5] = 2;
    assert!(read_bytes(&v2, "x").unwrap_err().contains("2.0"), "version 2.0");
    // Too many colors, or too many groups (empty group starts take 6 bytes each).
    let colors: Vec<(u16, Vec<u8>)> = (0..=ASE_MAX_ENTRIES).map(|_| ase_color("Gray", b"Gray", &[0.5], 2)).collect();
    assert!(read_bytes(&ase(&colors), "x").unwrap_err().contains("more than"));
    let groups: Vec<(u16, Vec<u8>)> = (0..=ASE_MAX_ENTRIES).map(|_| (ASE_GROUP_START, vec![])).collect();
    assert!(read_bytes(&ase(&groups), "x").unwrap_err().contains("more than"));
    assert_eq!(read_bytes(&ase(&groups[1..]), "x").unwrap().groups.len(), ASE_MAX_ENTRIES, "up to the limit");
}

#[test]
fn ase_writes_each_model_kind_and_group() {
    let sw = |name: &str, color, global, spot| Swatch { name: name.into(), paint: Paint::solid(color), global, spot };
    let tint = Swatch {
        name: "Ink 40%".into(),
        paint: Paint::Solid { color: Color::cmyk(0.4, 0.2, 0.0, 0.08), swatch: Some("Ink".into()), tint: 0.4 },
        global: false,
        spot: false,
    };
    // `sample()` holds the spot color Ink, the gradient Fade, the pattern Tiles and the group Neutrals.
    let mut lib = sample();
    lib.swatches.insert(0, sw("Sky", Color::rgb(0.0, 0.5, 1.0), true, false));
    lib.swatches.push(tint);
    lib.groups[0].swatches = vec![sw("Mist", Color::gray(0.25), false, false), sw("Clay", Color::lab(50.0, 20.0, -30.0), false, false)];
    lib.groups.push(SwatchGroup { name: "Empty".into(), swatches: vec![] });
    let bytes = write(&lib, PaletteFormat::Ase).unwrap();
    let expected = ase(&[
        ase_color("Sky", b"RGB ", &[0.0, 0.5, 1.0], 0),
        ase_color("Ink", b"CMYK", &[1.0, 0.5, 0.0, 0.2], 1),
        ase_color("Ink 40%", b"CMYK", &[0.4, 0.2, 0.0, 0.08], 2),
        (ASE_GROUP_START, ase_name("Neutrals")),
        ase_color("Mist", b"Gray", &[0.75], 2),
        ase_color("Clay", b"LAB ", &[0.5, 20.0, -30.0], 2),
        (ASE_GROUP_END, vec![]),
        (ASE_GROUP_START, ase_name("Empty")),
        (ASE_GROUP_END, vec![]),
    ]);
    assert_eq!(bytes, expected, "no gradient or pattern; the tint as the process color it shows");
    let back = read_bytes(&bytes, "Brand").unwrap();
    assert_eq!((back.len(), back.groups.len()), (5, 2), "the empty group stays");
    assert_eq!(PaletteFormat::parse(".ASE"), Some(PaletteFormat::Ase));
    assert!(PaletteFormat::Ase.readable() && PaletteFormat::Ase.binary() && !PaletteFormat::Gpl.binary());
}

#[test]
fn ase_names_lose_nul_and_stop_at_the_reader_limit() {
    let names = ["Re\0d".to_string(), "é".repeat(1030), format!("a{}", "🎨".repeat(600))];
    let swatches = names.iter().map(|n| Swatch { name: n.clone(), paint: Paint::solid(Color::BLACK), global: false, spot: false }).collect();
    let lib = SwatchLibrary { name: "x".into(), swatches, groups: vec![] };
    let back = read_bytes(&write(&lib, PaletteFormat::Ase).unwrap(), "x").unwrap();
    let got: Vec<&str> = back.swatches.iter().map(|w| w.name.as_str()).collect();
    // At most 1024 UTF-16 code units, cut between characters: "a" and 511 two-unit emoji.
    assert_eq!(got, ["Red".to_string(), "é".repeat(1024), format!("a{}", "🎨".repeat(511))]);
}

#[test]
fn ase_clamps_components_and_writes_non_finite_ones_as_zero() {
    let sw = |name: &str, color| Swatch { name: name.into(), paint: Paint::solid(color), global: false, spot: false };
    let swatches = vec![
        sw("Lab", Color::lab(100.5, 300.0, -300.0)),
        sw("Rgb", Color::rgb(2.0, -1.0, f32::NAN)),
        sw("Cmyk", Color::cmyk(1.5, 0.0, 0.0, f32::INFINITY)),
        sw("Gray", Color::gray(1.5)),
    ];
    let bytes = write(&SwatchLibrary { name: "x".into(), swatches, groups: vec![] }, PaletteFormat::Ase).unwrap();
    let expected = ase(&[
        ase_color("Lab", b"LAB ", &[1.0, 127.0, -128.0], 2),
        ase_color("Rgb", b"RGB ", &[1.0, 0.0, 0.0], 2),
        ase_color("Cmyk", b"CMYK", &[1.0, 0.0, 0.0, 0.0], 2),
        ase_color("Gray", b"Gray", &[0.0], 2),
    ]);
    assert_eq!(bytes, expected, "clamped to the ranges .ase holds, a component that isn't finite as 0");
    // Lightness 100.5 is written as 1, which reads back as L* 100.
    let back = read_bytes(&bytes, "x").unwrap();
    assert_eq!(back.swatches[0].paint.color(), Some(Color::lab(100.0, 127.0, -128.0)));
}

#[test]
fn ase_write_refuses_more_entries_than_the_reader_takes() {
    let gray = |i: usize| Swatch { name: format!("G{i}"), paint: Paint::solid(Color::gray(0.5)), global: false, spot: false };
    // The limit counts colors and groups: a group and `ASE_MAX_ENTRIES - 1` colors reach it.
    let mut lib = SwatchLibrary {
        name: "x".into(),
        swatches: (1..ASE_MAX_ENTRIES).map(gray).collect(),
        groups: vec![SwatchGroup { name: "Group".into(), swatches: vec![] }],
    };
    let back = read_bytes(&write(&lib, PaletteFormat::Ase).unwrap(), "x").unwrap();
    assert_eq!((back.len(), back.groups.len()), (ASE_MAX_ENTRIES - 1, 1));
    lib.swatches.push(gray(0));
    let e = write(&lib, PaletteFormat::Ase).unwrap_err();
    assert!(e.contains("reads at most 100000 colors and groups from a swatch exchange file, and this library has 100001"), "{e}");
    // A gradient is left out and is not counted.
    lib.swatches.last_mut().unwrap().paint = sample().swatches[1].paint.clone();
    assert!(write(&lib, PaletteFormat::Ase).is_ok());
}

/// A color in one of the four models, in the ranges `.ase` holds.
fn arb_color() -> impl Strategy<Value = Color> {
    let unit = || 0.0f32..=1.0;
    prop_oneof![
        (unit(), unit(), unit()).prop_map(|(r, g, b)| Color::rgb(r, g, b)),
        (unit(), unit(), unit(), unit()).prop_map(|(c, m, y, k)| Color::cmyk(c, m, y, k)),
        unit().prop_map(Color::gray),
        (0.0f32..=100.0, -128.0f32..=127.0, -128.0f32..=127.0).prop_map(|(l, a, b)| Color::lab(l, a, b)),
    ]
}

/// A swatch's paint with its global and spot flags: a solid color with any flags, a tint swatch
/// (both flags off) or a gradient.
fn arb_paint() -> impl Strategy<Value = (Paint, bool, bool)> {
    prop_oneof![
        4 => (arb_color(), any::<bool>(), any::<bool>()).prop_map(|(c, global, spot)| (Paint::solid(c), global, spot)),
        1 => arb_color().prop_map(|c| (Paint::Solid { color: c, swatch: Some("Base".into()), tint: 0.4 }, false, false)),
        1 => Just((sample().swatches[1].paint.clone(), false, false)),
    ]
}

/// Ungrouped swatches and color groups. A name is a number and up to 12 characters of any script
/// but NUL, trimmed, so swatches and groups have unique names, as in a document.
fn arb_library() -> impl Strategy<Value = SwatchLibrary> {
    let swatches = || prop::collection::vec((arb_paint(), "[^\\x00]{0,12}"), 0..12);
    (swatches(), prop::collection::vec(("[^\\x00]{0,12}", swatches()), 0..4)).prop_map(|(loose, groups)| {
        let mut n = 0;
        let mut name = |prefix: &str, base: &str| {
            n += 1;
            format!("{prefix}{n} {base}").trim().to_string()
        };
        let mut lib = SwatchLibrary { name: "Fuzz".into(), ..Default::default() };
        for ((paint, global, spot), base) in loose {
            lib.swatches.push(Swatch { name: name("", &base), paint, global, spot });
        }
        for (base, list) in groups {
            let mut g = SwatchGroup { name: name("G", &base), swatches: vec![] };
            for ((paint, global, spot), base) in list {
                g.swatches.push(Swatch { name: name("", &base), paint, global, spot });
            }
            lib.groups.push(g);
        }
        lib
    })
}

/// Are `got` and `want` in the same model, with every component within 1e-4? Lab lightness and
/// gray read back within `f32` rounding.
fn close(got: Color, want: Color) -> bool {
    let near = |a: f32, b: f32| (a - b).abs() < 1e-4;
    match (got, want) {
        (Color::Rgb { r, g, b }, Color::Rgb { r: r2, g: g2, b: b2 }) => near(r, r2) && near(g, g2) && near(b, b2),
        (Color::Cmyk { c, m, y, k }, Color::Cmyk { c: c2, m: m2, y: y2, k: k2 }) => near(c, c2) && near(m, m2) && near(y, y2) && near(k, k2),
        (Color::Gray { k }, Color::Gray { k: k2 }) => near(k, k2),
        (Color::Lab { l, a, b }, Color::Lab { l: l2, a: a2, b: b2 }) => near(l, l2) && near(a, a2) && near(b, b2),
        _ => false,
    }
}

/// The swatches of `list` as an `.ase` file reads them back: solid colors only, unlinked, spot
/// colors global.
fn ase_expected(list: &[Swatch]) -> Vec<Swatch> {
    list.iter()
        .filter_map(|w| Some(Swatch { name: w.name.clone(), paint: Paint::solid(w.paint.color()?), global: w.global || w.spot, spot: w.spot }))
        .collect()
}

/// Do swatches `got` have the names, flags and unlinked colors (within 1e-4) of `want`?
fn same_swatches(got: &[Swatch], want: &[Swatch]) -> bool {
    got.len() == want.len()
        && got.iter().zip(want).all(|(g, w)| {
            let color = match (&g.paint, w.paint.color()) {
                (Paint::Solid { color, swatch: None, .. }, Some(c)) => close(*color, c),
                _ => false,
            };
            g.name == w.name && g.global == w.global && g.spot == w.spot && color
        })
}

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

    /// A written `.ase` file reads back as the library's solid colors in their own model, with
    /// their names, kinds and color groups.
    #[test]
    fn ase_round_trip_keeps_models_kinds_and_groups(lib in arb_library()) {
        let bytes = write(&lib, PaletteFormat::Ase).unwrap();
        prop_assert!(sniff_bytes(&bytes));
        let back = read_bytes(&bytes, &lib.name).unwrap();
        prop_assert_eq!(&back.name, &lib.name);
        prop_assert!(same_swatches(&back.swatches, &ase_expected(&lib.swatches)), "{:?}", back.swatches);
        prop_assert_eq!(back.groups.len(), lib.groups.len());
        for (g, want) in back.groups.iter().zip(&lib.groups) {
            prop_assert_eq!(&g.name, &want.name);
            prop_assert!(same_swatches(&g.swatches, &ase_expected(&want.swatches)), "{}: {:?}", g.name, g.swatches);
        }
    }

    /// A written `.vcswatches`, `.gpl` or `.ase` file reads back with as many swatches as
    /// [`PaletteFormat::holds`] counts.
    #[test]
    fn written_files_read_back_with_the_swatches_holds_counts(lib in arb_library()) {
        for f in [PaletteFormat::Native, PaletteFormat::Gpl, PaletteFormat::Ase] {
            let back = read_bytes(&write(&lib, f).unwrap(), &lib.name).unwrap();
            prop_assert_eq!(back.len(), lib.iter().filter(|w| f.holds(&w.paint)).count(), "{:?}", f);
        }
    }
}
