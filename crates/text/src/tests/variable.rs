//! Variable fonts: each named instance is a style of its own, listed, resolved, shaped, drawn and
//! embedded at its axis settings (#296: Figtree listed only Light, its default instance).

use kurbo::{Point, Shape};
use skrifa::MetadataProvider;
use skrifa::instance::{LocationRef, Size};
use skrifa::raw::TableProvider;
use vectorcraft_doc::CharStyle;

use crate::test_fonts::{VARIABLE_CHARS, VARIABLE_FAMILY as FAMILY, VARIABLE_WIDEN, name_table, variable_font};
use crate::*;

fn font() -> Vec<u8> {
    variable_font().expect("the test font builds")
}

/// A database of the bundled fonts and the test variable font.
fn db() -> FontDb {
    let db = FontDb::with_font_dirs(vec![]);
    assert_eq!(db.add_font(font()), 3, "Regular, SemiBold and Bold");
    db
}

/// The width of `c`'s outline in font units.
fn ink_width(db: &FontDb, face: &FontFace, c: char) -> f64 {
    db.outline(face, face.glyph_for(c)).bounding_box().width()
}

fn text(family: &str, style: &str, s: &str) -> TextObject {
    TextObject::point(Point::ZERO, s, CharStyle { font_family: family.into(), font_style: style.into(), size: 100.0, ..CharStyle::default() })
}

#[test]
fn named_instances_are_styles_of_the_family() {
    let db = db();
    // The default instance is the face itself; the unnamed and the twice-named ones are left out.
    assert_eq!(db.styles(FAMILY), ["Regular", "SemiBold", "Bold"]);
    let bold = db.face(FAMILY, "Bold").unwrap();
    assert_eq!((bold.family.as_str(), bold.style.as_str(), bold.weight, bold.italic), (FAMILY, "Bold", 700.0, false));
    assert_eq!(bold.variations(), [(*b"wght", 700.0)]);
    let regular = db.face(FAMILY, "Regular").unwrap();
    assert!(regular.variations().is_empty() && regular.weight == 400.0);
    assert_eq!(db.face(FAMILY, "SemiBold").unwrap().weight, 550.0);
    // Fonts added again add nothing.
    assert_eq!(db.add_font(font()), 0);
}

#[test]
fn an_instance_is_found_by_its_names_and_by_weight() {
    let db = db();
    let m = |f: &str, s: &str| db.resolve(f, s).map(|(face, m)| (face.style.clone(), m)).unwrap();
    assert_eq!(m(FAMILY, "Bold"), ("Bold".into(), FontMatch::Exact));
    assert_eq!(m("varitest sans", "semi bold"), ("SemiBold".into(), FontMatch::Exact), "names are normalized");
    // The default instance's own name names the face.
    assert_eq!(m(FAMILY, "Normal"), ("Regular".into(), FontMatch::Exact));
    assert!(!db.styles(FAMILY).contains(&"Normal".to_string()));
    // A style the font hasn't: the closest weight.
    assert_eq!(m(FAMILY, "Black"), ("Bold".into(), FontMatch::Style));
    assert_eq!(m(FAMILY, "Medium"), ("SemiBold".into(), FontMatch::Style));
    // PostScript names (as PDF and .ai files name fonts): `<family>-<style>` without spaces.
    assert_eq!(m("VaritestSans-Bold", "Regular"), ("Bold".into(), FontMatch::Exact));
    assert_eq!(db.find_postscript("VaritestSans-SemiBold").unwrap().style, "SemiBold");
}

#[test]
fn an_instance_draws_and_measures_its_own_glyphs() {
    let db = db();
    let (regular, semi, bold) = (db.face(FAMILY, "Regular").unwrap(), db.face(FAMILY, "SemiBold").unwrap(), db.face(FAMILY, "Bold").unwrap());
    let widen = f64::from(VARIABLE_WIDEN);
    for c in VARIABLE_CHARS.chars() {
        let g = regular.glyph_for(c);
        assert_eq!(bold.glyph_for(c), g);
        // Advances (gvar phantom points) and outlines: a quarter wider at Bold, half that at SemiBold.
        let (r, b, s) = (regular.advance(g), bold.advance(g), semi.advance(g));
        assert!((b - r * (1.0 + widen)).abs() <= 1.0, "{c}: advance {r} -> {b}");
        assert!((s - r * (1.0 + widen / 2.0)).abs() <= 1.0, "{c}: SemiBold advance {s}");
        let (ri, bi) = (ink_width(&db, &regular, c), ink_width(&db, &bold, c));
        assert!((bi - ri * (1.0 + widen)).abs() <= 2.0, "{c}: outline {ri} -> {bi}");
    }
    // Characters the axis doesn't change stay as they are.
    let o = regular.glyph_for('o');
    assert_eq!(regular.advance(o), bold.advance(o));
    assert_eq!(db.outline(&regular, o).to_svg(), db.outline(&bold, o).to_svg());

    // Laid out (shaped with the instance): the glyphs come from the Bold face, advance as it does
    // and are drawn with its outlines (what the canvas, Create Outlines and hit tests use).
    let (lr, lb) = (layout(&db, &text(FAMILY, "Regular", VARIABLE_CHARS)), layout(&db, &text(FAMILY, "Bold", VARIABLE_CHARS)));
    assert!(lb.glyphs.iter().all(|g| g.font_id == bold.id()));
    let width = |l: &TextLayout| l.glyphs.iter().map(|g| g.advance).sum::<f64>();
    let expected: f64 = VARIABLE_CHARS.chars().map(|c| bold.advance(bold.glyph_for(c))).sum::<f64>() * 100.0 / bold.units_per_em();
    assert!((width(&lb) - expected).abs() < 0.5, "shaped {} vs {expected}", width(&lb));
    assert!(width(&lb) > width(&lr) * (1.0 + widen * 0.9));
    let ink = |l: &TextLayout| l.to_bezpath().bounding_box().width();
    assert!(ink(&lb) > ink(&lr) * (1.0 + widen * 0.9), "{} vs {}", ink(&lb), ink(&lr));
}

#[test]
fn an_embedded_instance_is_a_static_font_of_that_instance() {
    let db = db();
    let bold = db.face(FAMILY, "Bold").unwrap();
    let e = bold.embed(&['l', 'o']).unwrap();
    assert!(e.subset && !e.cff);
    let f = skrifa::FontRef::new(&e.data).unwrap();
    assert!(f.fvar().is_err() && f.gvar().is_err(), "no variations left");
    let g = f.charmap().map('l').unwrap();
    let adv = f.glyph_metrics(Size::unscaled(), LocationRef::default()).advance_width(g).unwrap();
    assert!((f64::from(adv) - bold.advance(bold.glyph_for('l'))).abs() <= 1.0, "the Bold advance: {adv}");
    // The default face embeds as before.
    let regular = db.face(FAMILY, "Regular").unwrap();
    let data = regular.embed(&['l']).unwrap().data;
    let r = skrifa::FontRef::new(&data).unwrap();
    let rl = r.glyph_metrics(Size::unscaled(), LocationRef::default()).advance_width(r.charmap().map('l').unwrap()).unwrap();
    assert!((f64::from(rl) - regular.advance(regular.glyph_for('l'))).abs() <= 1.0 && rl < adv, "{rl} vs {adv}");
}

#[test]
fn installed_variable_fonts_list_their_instances_before_loading() {
    let dir = std::env::temp_dir().join(format!("vc-varfonts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Varitest[wght].ttf"), font()).unwrap();
    let db = FontDb::with_font_dirs(vec![dir]);
    assert_eq!(db.styles(FAMILY), ["Regular", "SemiBold", "Bold"]);
    assert!(!db.is_loaded(FAMILY), "listed from the name and fvar tables alone");
    assert_eq!(db.by_postscript_name("VaritestSans-Bold"), Some((FAMILY.to_string(), "Bold".to_string())));
    let (bold, m) = db.resolve(FAMILY, "Bold").unwrap();
    assert_eq!((bold.style.as_str(), m), ("Bold", FontMatch::Exact));
    assert_eq!(bold.variations(), [(*b"wght", 700.0)]);
    assert!(bold.path().is_some());
}

/// Damaged or hostile `fvar` tables give fewer styles, never a panic, and the face still works.
#[test]
fn damaged_variation_tables_never_panic() {
    let name = name_table(&[(1, "Broken Var"), (2, "Regular"), (256, "W")]).unwrap();
    let be = |v: &[u32]| v.iter().flat_map(|x| x.to_be_bytes()).collect::<Vec<u8>>();
    // Header: version 1.0, axes offset 16, reserved, axis count, axis size 20, instance count,
    // instance size.
    let header = |axes: u16, axis_size: u16, instances: u16, instance_size: u16| {
        let mut h = be(&[0x0001_0000]);
        for v in [16, 2, axes, axis_size, instances, instance_size] {
            h.extend(v.to_be_bytes());
        }
        h
    };
    let axis = |tag: &[u8; 4], min: i32, def: i32, max: i32| {
        let mut a = tag.to_vec();
        a.extend(be(&[(min << 16) as u32, (def << 16) as u32, (max << 16) as u32]));
        a.extend([0, 0, 1, 0]);
        a
    };
    let mut damaged: Vec<Vec<u8>> = vec![
        vec![],
        vec![0; 3],
        header(0xffff, 20, 0xffff, 8),
        [header(1, 20, 0xffff, 8), axis(b"wght", 100, 400, 900)].concat(),
        [header(1, 20, 3, 0xffff), axis(b"wght", 100, 400, 900)].concat(),
        [header(1, 0, 3, 8), axis(b"wght", 100, 400, 900)].concat(),
        [header(200, 20, 1, 804), vec![0; 4000]].concat(),
    ];
    // Many instances (all named): the list is capped.
    let mut many = [header(1, 20, 2000, 8), axis(b"wght", 0, 0, 30000)].concat();
    for i in 0..2000u32 {
        many.extend(be(&[256 << 16, (i + 1) << 16]));
    }
    damaged.push(many);
    for fvar in &damaged {
        let f = fontdb::sfnt_of(&[(b"name", &name), (b"fvar", fvar)]).unwrap();
        let styles = fontdb::face_styles(&skrifa::FontRef::new(&f).unwrap());
        assert!(!styles.is_empty() && styles.len() <= 2, "{} styles", styles.len());
    }
    // A real font with a damaged fvar (and its gvar) still loads and lays out.
    let good = font();
    let at = |tag: &[u8; 4]| {
        let tables = u16::from_be_bytes([good[4], good[5]]) as usize;
        let r = (0..tables).map(|i| 12 + 16 * i).find(|r| &good[*r..*r + 4] == tag).unwrap();
        let off = u32::from_be_bytes(good[r + 8..r + 12].try_into().unwrap()) as usize;
        let len = u32::from_be_bytes(good[r + 12..r + 16].try_into().unwrap()) as usize;
        off..off + len
    };
    let (fvar, gvar) = (at(b"fvar"), at(b"gvar"));
    // Deterministic byte damage across both tables.
    let mut seed = 0x2545_f491_u32;
    for round in 0..300 {
        let mut bad = good.clone();
        for _ in 0..1 + round % 8 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let range = if seed.is_multiple_of(2) { fvar.clone() } else { gvar.clone() };
            let i = range.start + (seed as usize >> 3) % range.len();
            bad[i] = (seed >> 24) as u8;
        }
        let db = FontDb::with_font_dirs(vec![]);
        db.add_font(bad);
        for style in db.styles(FAMILY) {
            let l = layout(&db, &text(FAMILY, &style, "lHio"));
            assert!(l.glyphs.len() <= 8, "{style}: {} glyphs", l.glyphs.len());
        }
    }
}

/// Windows ships variable system fonts: Bahnschrift (weight and width axes) lists its named
/// instances and draws them. Skipped where it isn't installed.
#[test]
fn bahnschrift_lists_and_draws_its_instances() {
    let db = FontDb::with_font_dirs(system_font_dirs());
    if !db.has_family("Bahnschrift") {
        eprintln!("skipped: Bahnschrift is not installed");
        return;
    }
    let styles = db.styles("Bahnschrift");
    eprintln!("Bahnschrift: {styles:?}");
    for s in ["Light", "Regular", "SemiBold", "Bold"] {
        assert!(styles.iter().any(|x| x == s), "{s} in {styles:?}");
    }
    let light = db.face("Bahnschrift", "Light").unwrap();
    let bold = db.face("Bahnschrift", "Bold").unwrap();
    assert_eq!((light.style.as_str(), bold.style.as_str()), ("Light", "Bold"));
    assert!(bold.weight > light.weight, "{} vs {}", bold.weight, light.weight);
    let (lw, bw) = (ink_width(&db, &light, 'l'), ink_width(&db, &bold, 'l'));
    assert!(bw > lw * 1.05, "the Bold l is wider: {lw} -> {bw}");
    let w = |s: &str| layout(&db, &text("Bahnschrift", s, "Hamburgefonstiv")).to_bezpath().bounding_box().width();
    assert!(w("Bold") > w("Light"), "{} vs {}", w("Bold"), w("Light"));
}

/// Segoe UI Variable (Windows 11; optical size and weight axes). Skipped where it isn't installed.
#[test]
fn segoe_ui_variable_lists_its_instances() {
    let db = FontDb::with_font_dirs(system_font_dirs());
    let families: Vec<String> = db.families().into_iter().filter(|f| f.starts_with("Segoe UI Variable")).collect();
    if families.is_empty() {
        eprintln!("skipped: Segoe UI Variable is not installed");
        return;
    }
    for f in &families {
        let styles = db.styles(f);
        eprintln!("{f}: {styles:?}");
        assert!(styles.len() > 1, "{f}: {styles:?}");
    }
    if families.iter().any(|f| f == "Segoe UI Variable") {
        let (light, bold) = (db.face("Segoe UI Variable", "Light Text").unwrap(), db.face("Segoe UI Variable", "Bold Text").unwrap());
        assert_eq!((light.style.as_str(), bold.style.as_str()), ("Light Text", "Bold Text"));
        assert!(bold.weight > light.weight && !bold.variations().is_empty());
        let w = |f: &FontFace| ink_width(&db, f, 'm');
        assert!(w(&bold) > w(&light), "{} vs {}", w(&bold), w(&light));
    }
}

/// #1258: a variable font's full name identifies the default face, not a named weight.
#[test]
fn a_variable_full_name_names_its_default_face() {
    let bytes = font();
    let source = skrifa::FontRef::new(&bytes).unwrap();
    let names = name_table(&[
        (1, FAMILY),
        (2, "Regular"),
        (4, "Varitest Default Face"),
        (6, "VaritestSans-Regular"),
        (256, "Weight"),
        (257, "Normal"),
        (258, "SemiBold"),
        (259, "Bold"),
        (261, "BOLD"),
    ])
    .unwrap();
    let mut builder = write_fonts::FontBuilder::new();
    builder.add_raw(write_fonts::types::Tag::new(b"name"), names);
    for record in source.table_directory.table_records() {
        let tag = write_fonts::types::Tag::new(&record.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, source.table_data(record.tag()).unwrap().as_bytes());
        }
    }
    let db = FontDb::with_font_dirs(vec![]);
    assert_eq!(db.add_font(builder.build()), 3);
    let (face, matched) = db.resolve("Varitest Default Face", "Bold").unwrap();
    assert_eq!((face.family.as_str(), face.style.as_str(), matched), (FAMILY, "Regular", FontMatch::Exact));
    assert!(face.variations().is_empty());
    let bold = db.resolve("VaritestSans-Bold", "Regular").unwrap().0;
    assert_eq!(bold.variations(), [(*b"wght", 700.0)]);
}

/// #2491: a variable font whose name table calls the default face `Light` (and whose OS/2 weight
/// says 300) while its default instance is Regular at `wght` 400 lists, resolves and weighs the
/// default face as Regular, as it renders.
#[test]
fn a_variable_default_face_is_named_and_weighed_by_its_default_instance() {
    let bytes = font();
    let source = skrifa::FontRef::new(&bytes).unwrap();
    let family = "Hebtest Sans";
    let names = name_table(&[
        (1, "Hebtest Sans Light"),
        (2, "Regular"),
        (4, "Hebtest Sans Light"),
        (6, "HebtestSans-Light"),
        (16, family),
        (17, "Light"),
        (256, "Weight"),
        (257, "Regular"),
        (258, "SemiBold"),
        (259, "Bold"),
    ])
    .unwrap();
    let mut os2 = source.table_data(skrifa::raw::types::Tag::new(b"OS/2")).unwrap().as_bytes().to_vec();
    os2[4..6].copy_from_slice(&300_u16.to_be_bytes());
    let mut builder = write_fonts::FontBuilder::new();
    builder.add_raw(write_fonts::types::Tag::new(b"name"), names);
    builder.add_raw(write_fonts::types::Tag::new(b"OS/2"), os2);
    for record in source.table_directory.table_records() {
        let tag = write_fonts::types::Tag::new(&record.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, source.table_data(record.tag()).unwrap().as_bytes());
        }
    }
    let db = FontDb::with_font_dirs(vec![]);
    assert_eq!(db.add_font(builder.build()), 3);
    assert_eq!(db.styles(family), ["Regular", "SemiBold", "Bold"]);
    let (face, matched) = db.resolve(family, "Regular").unwrap();
    assert_eq!((face.style.as_str(), face.weight, matched), ("Regular", 400.0, FontMatch::Exact));
    assert!(face.variations().is_empty());
}
