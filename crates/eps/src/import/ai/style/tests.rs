use super::*;

#[test]
fn a_variable_width_stroke_keeps_its_profile() {
    let data = b"/KnownStyle :\n(Tapered) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
            /CompoundFilter :\n(Stack Style Filter) 0 0 /Filter ,\n1 /Visible ,\n/BasicFilter :\n(Stroke Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
            2 /FillOrStroke ,\n/Dictionary : /NotRecorded ,\n/StrokeStyle : 0 R\n0 0 0 0 0 0 1 XA\n0 J 0 j 30 w 10 M []0 d\n0 XR\n1 0 Xd\n/Def ;\n (StrokeStyle) ,\n\
            /Dictionary : /NotRecorded ,\n2 /Int (BSWidthSource) ,\n/Array :\n0 /Real ,\n1 /Real ,\n; (BSVariableWidthTValuesArray) ,\n\
            /Array :\n1 /Real ,\n0.1 /Real ,\n; (BSCenteredWidthsArray) ,\n; (kAIBeautifulStrokesParamsDictKey) ,\n; /Dict ;\n /Part ,\n\
            /BasicFilter :\n(Conduit Filter) 0 0 /Filter ,\n1 /Visible ,\n;\n /Part ,\n;\n";
    let styles = art_styles(data);
    let style = &styles["Tapered"];
    assert!(style.plain);
    let [StyleItem::Stroke { width, profile, .. }] = style.items.as_slice() else { panic!("{:?}", style.items) };
    assert_eq!(*width, 30.0);
    assert_eq!(profile.as_ref().map(|p| p.points.clone()), Some(vec![(0.0, 1.0, 1.0), (1.0, 0.1, 0.1)]));
}

#[test]
fn dashes_aligned_to_corners_keep_the_drawn_look() {
    // GUARDS: the look holds the fitted pattern and the solid ends the style's dash array doesn't.
    let data = b"/KnownStyle :\n(Stitches) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
            /BasicFilter :\n(Stroke Style Filter) 0 0 /Filter ,\n1 /Visible ,\n2 /FillOrStroke ,\n/Dictionary : /NotRecorded ,\n\
            /Dictionary : /NotRecorded ,\n1 /Bool (BSDashAdj) ,\n; (kAIBeautifulStrokesParamsDictKey) ,\n\
            /StrokeStyle : 0 R\n0 0 0 0 0 0 1 XA\n1 J 0 j 2 w 10 M [5 10 ]0 d\n0 XR\n1 0 Xd\n/Def ;\n (StrokeStyle) ,\n; /Dict ;\n /Part ,\n;\n";
    assert!(!art_styles(data)["Stitches"].plain);
}

/// The style `Shade`: a Drop Shadow whose dictionary holds `keys`.
fn shadow_style(keys: &str) -> ArtStyle {
    effect_style("Adobe Drop Shadow", keys)
}

/// The style `Fx`: an effect `name` on the whole object, its dictionary holding `keys`.
fn effect_style(name: &str, keys: &str) -> ArtStyle {
    let data = format!(
        "/KnownStyle :\n(Fx) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
            /BasicFilter :\n({name}) 1 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n{keys}; /Dict ;\n /Part ,\n;\n"
    );
    art_styles(data.as_bytes()).remove("Fx").unwrap_or_default()
}

/// A colour entry `key`: a fill style in RGB `r g b`.
fn color_entry(key: &str, rgb: &str) -> String {
    format!("/FillStyle : 0 O\n0 0 0 0 {rgb} Xa\n0 1 0 0 0 Xy\n0 J 0 j 1 w 10 M []0 d\n0 XR\n0 1 Xd\n/Def ;\n ({key}) ,\n")
}

/// A Drop Shadow's dictionary: every setting, with Color or Darkness set by `shade`.
fn shadow_keys(shade: &str) -> String {
    format!(
        "2 /Int (blnd) ,\n0.43 /Real (opac) ,\n37 /Real (dark) ,\n-13 /Real (vert) ,\n11 /Real (horz) ,\n17 /Real (blur) ,\n{}{shade}",
        color_entry("sclr", "0 1 0")
    )
}

/// `style`'s one effect on the whole object.
fn only_effect(style: &ArtStyle) -> &Effect {
    assert!(style.plain, "{style:?}");
    let [e] = style.effects.as_slice() else { panic!("{style:?}") };
    e
}

/// `params[key]`, a number within rounding of `want`.
fn near(params: &serde_json::Value, key: &str, want: f64) {
    let got = params[key].as_f64();
    assert!(got.is_some_and(|v| (v - want).abs() < 1e-9), "{key}: {got:?}, not {want}: {params}");
}

#[test]
fn a_drop_shadow_reads_its_settings() {
    // GUARDS: `vert` read as an art-space y would flip a shadow offset down to up.
    let style = shadow_style(&shadow_keys("0 /Int (csrc) ,\n"));
    let shadow = only_effect(&style);
    let p = &shadow.params;
    assert_eq!(
        (shadow.id.as_str(), &p["mode"], &p["color"]),
        ("stylize.dropShadow", &json!("Screen"), &json!(vectorcraft_color::Color::rgb(0.0, 1.0, 0.0)))
    );
    for (key, want) in [("x", 11.0), ("y", -13.0), ("blur", 34.0), ("opacity", 43.0)] {
        near(p, key, want);
    }
    assert_eq!((p.get("shade"), p.get("darkness")), (None, None));
}

#[test]
fn a_drop_shadow_with_darkness_reads_its_darkness() {
    let shadow = only_effect(&shadow_style(&shadow_keys("1 /Int (csrc) ,\n"))).clone();
    assert_eq!((&shadow.params["shade"], &shadow.params["darkness"]), (&json!("darkness"), &json!(37.0)));
}

#[test]
fn a_drop_shadow_with_settings_not_read_keeps_the_drawn_look() {
    // GUARDS: an unknown `csrc` was read as Darkness, `blnd` was cast to a mode, a missing setting
    // took a default and an unknown key was skipped: each became an editable shadow the file doesn't draw.
    let full = shadow_keys("0 /Int (csrc) ,\n");
    let cases = [
        full.replace("0 /Int (csrc)", "99 /Int (csrc)"),
        full.replace("2 /Int (blnd)", "99 /Int (blnd)"),
        full.replace("2 /Int (blnd)", "2.5 /Int (blnd)"),
        full.replace("0 /Int (csrc)", "0.5 /Int (csrc)"),
        full.replace("11 /Real (horz)", "11 /Int (horz)"),
        full.replace("37 /Real (dark)", "37 /Int (dark)"),
        full.replace("2 /Int (blnd)", "2 /Real (blnd)"),
        full.replace("0.43 /Real (opac)", "1.5 /Real (opac)"),
        full.replace("11 /Real (horz) ,\n", ""),
        full.replace(&color_entry("sclr", "0 1 0"), ""),
        full.replace("0 1 0 0 0 Xy", "2 0.5 0 0 0 Xy"),
        format!("{full}1 /Real (spread) ,\n"),
        format!("{full}0 /Bool (pair) ,\n"),
        format!("{full}(x) /String (note) ,\n"),
        format!("{full}3 4 /RealPointRelToROrigin (spot) ,\n"),
        shadow_keys("1 /Int (csrc) ,\n").replace("37 /Real (dark)", "250 /Real (dark)"),
        shadow_keys("1 /Int (csrc) ,\n").replace("37 /Real (dark)", "(fifty) /Real (dark)"),
    ];
    for keys in cases {
        assert!(!shadow_style(&keys).plain, "{keys}");
    }
    let observed = format!("{full}1 /Bool (pair) ,\n1 /Bool (usePSLBlur) ,\n16 /Int (Adobe Effect Expand Before Version) ,\n");
    assert!(shadow_style(&observed).plain);
}

#[test]
fn an_inner_glow_reads_its_settings() {
    let keys =
        |gtyp: &str| format!("10 /Int (blnd) ,\n0.41 /Real (opac) ,\n13 /Real (blur) ,\n{}{gtyp} /Int (gtyp) ,\n", color_entry("gclr", "1 0 0"));
    let style = effect_style("Adobe Inner Glow", &keys("0"));
    let glow = only_effect(&style);
    let p = &glow.params;
    assert_eq!(
        (glow.id.as_str(), &p["mode"], &p["source"], &p["color"]),
        ("stylize.innerGlow", &json!("Difference"), &json!("center"), &json!(vectorcraft_color::Color::rgb(1.0, 0.0, 0.0)))
    );
    near(p, "opacity", 41.0);
    near(p, "blur", 13.0);
    assert!(style.simple);
    assert_eq!(only_effect(&effect_style("Adobe Inner Glow", &keys("1"))).params["source"], json!("edge"));
    assert!(!effect_style("Adobe Inner Glow", &keys("2")).plain);
}

#[test]
fn an_outer_glow_reads_its_settings_and_keeps_the_drawn_look_for_others() {
    let keys = format!(
        "16 /Int (Adobe Effect Expand Before Version) ,\n10 /Int (blnd) ,\n0.42 /Real (opac) ,\n19 /Real (blur) ,\n{}1 /Bool (usePSLBlur) ,\n",
        color_entry("sclr", "0 0 1")
    );
    let glow = only_effect(&effect_style("Adobe Outer Glow", &keys)).clone();
    assert_eq!((glow.id.as_str(), &glow.params["mode"]), ("stylize.outerGlow", &json!("Difference")));
    near(&glow.params, "opacity", 42.0);
    near(&glow.params, "blur", 38.0);
    for other in [
        format!("{keys}1 /Real (spread) ,\n"),
        keys.replace("19 /Real (blur)", "19 /Int (blur)"),
        keys.replace("1 /Bool (usePSLBlur)", "0 /Bool (usePSLBlur)"),
    ] {
        assert!(!effect_style("Adobe Outer Glow", &other).plain, "{other}");
    }
}

#[test]
fn a_feather_reads_its_radius() {
    // GUARDS: Feather was looked for under another name and key, so the files' Feather kept the drawn look.
    let style = effect_style("Adobe Fuzzy Mask", "14 /Real (Radius) ,\n");
    let feather = only_effect(&style);
    assert_eq!((feather.id.as_str(), feather.params["radius"].as_f64()), ("stylize.feather", Some(14.0)));
    assert!(style.simple);
    for other in ["14 /Real (radius) ,\n", "", "-1 /Real (Radius) ,\n"] {
        assert!(!effect_style("Adobe Fuzzy Mask", other).plain, "{other}");
    }
}

#[test]
fn round_corners_read_their_radius() {
    let style = effect_style("Adobe Round Corners", "21 /Real (radius) ,\n");
    let round = only_effect(&style);
    assert_eq!((round.id.as_str(), round.params["radius"].as_f64()), ("stylize.roundCorners", Some(21.0)));
    assert!(style.simple);
    for other in ["21 /Real (Radius) ,\n", "21 /Int (radius) ,\n", "-1 /Real (radius) ,\n"] {
        assert!(!effect_style("Adobe Round Corners", other).plain, "{other}");
    }
}

/// A Scribble's dictionary with every setting.
const SCRIBBLE_KEYS: &str = "0.8 /Real (SpacingVariation) ,\n0.23 /Real (ScribbleVariation) ,\n4 /Real (EdgeOverlapVariation) ,\n67 /Real (Angle) ,\n\
    2.3 /Real (StrokeWidth) ,\n-7 /Real (EdgeOverlap) ,\n0.17 /Real (Scribbliness) ,\n13 /Real (Spacing) ,\n";

#[test]
fn a_scribble_reads_its_settings() {
    let style = effect_style("Adobe Scribble Fill", SCRIBBLE_KEYS);
    let scribble = only_effect(&style);
    assert_eq!(scribble.id, "stylize.scribble");
    let want = [
        ("angle", 67.0),
        ("overlap", -7.0),
        ("overlapVariation", 4.0),
        ("strokeWidth", 2.3),
        ("curviness", 17.0),
        ("curvinessVariation", 23.0),
        ("spacing", 13.0),
        ("spacingVariation", 0.8),
    ];
    for (key, v) in want {
        near(&scribble.params, key, v);
    }
    assert!(style.simple);
}

#[test]
fn a_scribble_with_a_colour_or_settings_not_read_keeps_the_drawn_look() {
    let cases = [
        format!("{SCRIBBLE_KEYS}{}", color_entry("sclr", "0 1 0")),
        format!("{SCRIBBLE_KEYS}0.5 /Real (MinDensity) ,\n"),
        SCRIBBLE_KEYS.replace("13 /Real (Spacing) ,\n", ""),
        SCRIBBLE_KEYS.replace("0.17 /Real (Scribbliness)", "17 /Real (Scribbliness)"),
    ];
    for keys in cases {
        assert!(!effect_style("Adobe Scribble Fill", &keys).plain, "{keys}");
    }
}

#[test]
fn effects_on_the_object_keep_their_order() {
    let data = format!(
        "/KnownStyle :\n(Two) /Name ,\n/ActiveStyle :\n{}",
        compound(
            "Chain Style Filter",
            &[basic("Adobe Round Corners", "10 /Real (radius) ,\n"), basic("Adobe Drop Shadow", &shadow_keys("0 /Int (csrc) ,\n"))]
        )
    );
    let st = &art_styles(data.as_bytes())["Two"];
    assert!(st.plain && st.simple, "{st:?}");
    assert_eq!(st.effects.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["stylize.roundCorners", "stylize.dropShadow"]);
}

#[test]
fn a_glow_on_one_fill_stays_on_that_fill() {
    // An ordinary object keeps the drawn look (`simple` is false); type takes the fill's glow.
    let glow = basic(
        "Adobe Inner Glow",
        &format!("2 /Int (blnd) ,\n0.75 /Real (opac) ,\n5 /Real (blur) ,\n{}1 /Int (gtyp) ,\n", color_entry("gclr", "1 1 1")),
    );
    let item = compound("Chain Style Filter", &[fill("0 0 0 1 0 0 0 Xa"), glow]);
    let data =
        format!("/KnownStyle :\n(Lit) /Name ,\n/ActiveStyle :\n{}", compound("Chain Style Filter", &[compound("Stack Style Filter", &[item])]));
    let st = &art_styles(data.as_bytes())["Lit"];
    assert!(st.plain && !st.simple, "{st:?}");
    assert!(st.effects.is_empty());
    assert_eq!(st.looks[0].effects.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["stylize.innerGlow"]);
}

#[test]
fn art_styles_clear_operands_beyond_the_cap() {
    let mut data = Vec::from(
        b"/KnownStyle :\n(Soft) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n/BasicFilter :\n(Adobe Fuzzy Mask) 1 0 /Filter ,\n1 /Visible ,\n",
    );
    for _ in 0..80 {
        data.extend_from_slice(b"1 ");
    }
    data.extend_from_slice(b"/Dictionary : /NotRecorded ,\n5 /Real (Radius) ,\n; /Dict ;\n /Part ,\n;\n");
    let styles = art_styles(&data);
    let [feather] = styles["Soft"].effects.as_slice() else { panic!() };
    assert_eq!(feather.params["radius"].as_f64(), Some(5.0));
}

#[test]
fn art_styles_ignore_filters_beyond_the_cap() {
    const UNIT: &[u8] =
        b"/BasicFilter :\n(Adobe Fuzzy Mask) 1 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n1 /Real (Radius) ,\n; /Dict ;\n /Part ,\n";
    let mut data =
        Vec::from(b"/KnownStyle :\n(Capped) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n");
    for _ in 0..MAX_FILTERS + 16 {
        data.extend_from_slice(UNIT);
    }
    data.extend_from_slice(b";\n");
    let style = art_styles(&data).remove("Capped").unwrap_or_default();
    assert!(style.effects.len() <= MAX_FILTERS);
}

#[test]
fn hostile_dash_offsets_and_width_profiles_are_tamed() {
    let mut f = Filter::default();
    paint_op(&mut f, "d", &[V::Arr(vec![V::Num(5.0)]), V::Num(f64::NAN)]);
    assert_eq!(f.dash.as_ref().map(|d| d.offset), Some(0.0));
    // Positions outside 0..=1 and out of order, a negative width, a non-finite one.
    let arrays = HashMap::from([
        ("BSVariableWidthTValuesArray".to_string(), vec![1.5, -0.2, 0.5, 0.7]),
        ("BSCenteredWidthsArray".to_string(), vec![-1.0, 2.0, 0.5, f64::INFINITY]),
    ]);
    assert_eq!(width_profile(&arrays).map(|p| p.points), Some(vec![(0.0, 2.0, 2.0), (0.5, 0.5, 0.5), (1.0, 0.0, 0.0)]));
    let ok = (0..64).map(|_| V::Num(2.0)).collect::<Vec<_>>();
    paint_op(&mut f, "d", &[V::Arr(ok), V::Num(0.0)]);
    assert_eq!(f.dash.as_ref().map(|d| d.pattern.len()), Some(64));
    let huge = (0..65).map(|_| V::Num(1.0)).collect::<Vec<_>>();
    paint_op(&mut f, "d", &[V::Arr(huge), V::Num(0.0)]);
    assert_eq!(f.dash, None);
    let long = (0..1025).map(|i| i as f64 / 1024.0).collect::<Vec<_>>();
    let wide = vec![1.0; 1025];
    let huge = HashMap::from([("BSVariableWidthTValuesArray".to_string(), long), ("BSCenteredWidthsArray".to_string(), wide)]);
    assert_eq!(width_profile(&huge), None);
}

#[test]
fn hostile_stroke_widths_are_sanitized() {
    let mut f = Filter::default();
    paint_op(&mut f, "w", &[V::Num(12.0)]);
    assert_eq!(f.width, 12.0);
    paint_op(&mut f, "w", &[V::Num(-1.0)]);
    assert_eq!(f.width, 12.0);
    paint_op(&mut f, "w", &[V::Num(f64::INFINITY)]);
    assert_eq!(f.width, 12.0);
}

#[test]
fn hostile_stroke_paint_flags_ignore_short_operands() {
    let mut f = Filter { paints: (true, true), ..Filter::default() };
    paint_op(&mut f, "Xd", &[V::Num(1.0)]);
    assert_eq!(f.paints, (true, true));
    paint_op(&mut f, "Xd", &[V::Num(0.0), V::Num(1.0)]);
    assert_eq!(f.paints, (false, true));
    paint_op(&mut f, "Xd", &[V::Num(2.0), V::Num(0.0)]);
    assert_eq!(f.paints, (true, false));
}

#[test]
fn hostile_stroke_miter_limits_are_capped() {
    let mut f = Filter::default();
    paint_op(&mut f, "M", &[V::Num(900.0)]);
    assert_eq!(f.miter, 500.0);
    paint_op(&mut f, "M", &[V::Num(0.5)]);
    assert_eq!(f.miter, 500.0);
}

#[test]
fn hostile_stroke_caps_and_joins_ignore_invalid_values() {
    let mut f = Filter::default();
    paint_op(&mut f, "J", &[V::Num(99.0)]);
    assert_eq!(f.cap, LineCap::Butt);
    paint_op(&mut f, "J", &[V::Num(2.0)]);
    assert_eq!(f.cap, LineCap::Square);
    paint_op(&mut f, "j", &[V::Num(99.0)]);
    assert_eq!(f.join, LineJoin::Miter);
    paint_op(&mut f, "j", &[V::Num(1.0)]);
    assert_eq!(f.join, LineJoin::Round);
}

#[test]
fn hostile_blend_opacities_are_clamped() {
    let mut f = Filter::default();
    paint_op(&mut f, "Xy", &[V::Num(3.0), V::Num(1.5)]);
    assert_eq!(f.xy, Some((3, 1.0)));
    paint_op(&mut f, "Xy", &[V::Num(-1.0), V::Num(0.25)]);
    assert_eq!(f.xy, Some((0, 0.25)));
    paint_op(&mut f, "Xy", &[V::Num(0.0), V::Num(f64::NAN)]);
    assert_eq!(f.xy, Some((0, 0.25)));
}

/// A basic filter `name` whose dictionary holds `body`.
fn basic(name: &str, body: &str) -> String {
    format!("/BasicFilter :\n({name}) 0 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n{body}; /Dict ;\n /Part ,\n")
}

/// A compound filter `name` holding `kids`.
fn compound(name: &str, kids: &[String]) -> String {
    format!("/CompoundFilter :\n({name}) 0 0 /Filter ,\n1 /Visible ,\n{};\n /Part ,\n", kids.concat())
}

fn fill(paint: &str) -> String {
    basic(
        "Fill Style Filter",
        &format!("0 /Bool (UseEvenOdd) ,\n/FillStyle : 0 O\n{paint}\n0 1 0 0 0 Xy\n0 J 0 j 1 w 10 M []0 d0 XR\n0 1 Xd\n/Def ;\n (FillStyle) ,\n"),
    )
}

fn blend(mode: u8, opacity: f64) -> String {
    basic("Blend Style Filter", &format!("/BlendStyle : {mode} {opacity} 0 0 0 Xy\n0 J 0 j 1 w 10 M []0 d0 XR\n/Def ;\n (BlendStyle) ,\n"))
}

/// A comic-lettering style: a warp over a stack of a textured fill, a gradient, a white rim
/// repeated by Transform, and the characters at the bottom.
fn lettering() -> String {
    let stack = compound(
        "Stack Style Filter",
        &[
            basic("Stroke Style Filter", "/StrokeStyle : 0 1 0 0 0 Xy\n0 J 0 j 1 w 10 M []0 d0 XR\n0 0 Xd\n/Def ;\n (StrokeStyle) ,\n"),
            compound(
                "Chain Style Filter",
                &[fill("(Dots) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p"), basic("PSAdapter_plugin_BsRl", "300 /Int (PrevDres) ,\n"), blend(5, 0.48)],
            ),
            fill("Bb\n0 0 0 0 Bh\n2 (Red ramp) 0 0 90 1 1 0 0 1 0 0 1 Bg\n0 BB"),
            compound(
                "Chain Style Filter",
                &[
                    basic("Adobe Offset Path", "3.5 /Real (ofst) ,\n0 /Int (jntp) ,\n4 /Real (mlim) ,\n"),
                    fill("0 0 0 0 1 1 1 Xa"),
                    basic(
                        "Adobe Transform",
                        "6 /Int (numCopies) ,\n0.75 /Real (moveH_Pts) ,\n-0.25 /Real (moveV_Pts) ,\n100 /Real (scaleH_Percent) ,\n100 /Real (scaleV_Percent) ,\n4 /Int (pinPoint) ,\n",
                    ),
                ],
            ),
            basic("Conduit Filter", ""),
        ],
    );
    let root = compound(
        "Chain Style Filter",
        &[
            basic(
                "Adobe Deform",
                "0 /Bool (Rotate) ,\n0.02 /Real (DeformVert) ,\n4 /Int (DeformStyle) ,\n0.05 /Real (DeformValue) ,\n0 /Real (DeformHoriz) ,\n",
            ),
            stack,
            blend(0, 1.0),
        ],
    );
    format!("/KnownStyle :\n(Lettering) /Name ,\n/ActiveStyle :\n{root}\n/Execution ;\n /Def ;\n")
}

#[test]
fn a_stack_of_fills_with_effects_of_their_own_is_read_as_itself() {
    // GUARDS: more than one fill, or an effect on a fill, made the style not plain, and type
    // with it came in as its drawn look (outlines) instead of type.
    let styles = art_styles(lettering().as_bytes());
    let st = &styles["Lettering"];
    assert!(st.plain, "{st:?}");
    // Bottom first: the rim, the gradient, the textured fill; the stroke paints nothing.
    let [StyleItem::Fill(rim), StyleItem::Gradient { name, angle }, StyleItem::Pattern { name: pattern, .. }] = st.items.as_slice() else {
        panic!("{:?}", st.items)
    };
    assert!(matches!(rim, Paint::Solid { .. }), "{rim:?}");
    assert_eq!((name.as_deref(), *angle, pattern.as_str()), (Some("Red ramp"), 90.0, "Dots"));
    let ids = |l: &ItemLook| l.effects.iter().map(|e| e.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&st.looks[0]), ["path.offsetPath", "distort.transform"]);
    assert!(ids(&st.looks[1]).is_empty());
    assert_eq!(ids(&st.looks[2]), ["sketch.basRelief"]);
    assert_eq!((st.looks[2].blend, st.looks[2].opacity), (BlendMode::HardLight, 0.48));
    let transform = &st.looks[0].effects[1].params;
    assert_eq!((&transform["copies"], &transform["moveH"], &transform["moveV"]), (&json!(6), &json!(0.75), &json!(0.25)));
    assert_eq!(st.looks[0].effects[0].params["joins"], json!("round"));
    // The warp is the whole object's; the characters paint below every item.
    let [warp] = st.effects.as_slice() else { panic!("{:?}", st.effects) };
    assert_eq!((warp.id.as_str(), &warp.params["bend"], &warp.params["orientation"]), ("warp.arch", &json!(5.0), &json!("horizontal")));
    assert_eq!(st.contents_index, None);
}

#[test]
fn contents_above_some_items_count_them() {
    let stack = compound("Stack Style Filter", &[fill("0 0 0 0 1 1 1 Xa"), basic("Conduit Filter", ""), fill("0 0 0 1 0 0 0 Xa")]);
    let data = format!("/KnownStyle :\n(Over) /Name ,\n/ActiveStyle :\n{}", compound("Chain Style Filter", &[stack]));
    let st = &art_styles(data.as_bytes())["Over"];
    assert!(st.plain, "{st:?}");
    assert_eq!((st.items.len(), st.contents_index), (2, Some(1)));
}

#[test]
fn an_effect_not_read_here_keeps_the_drawn_look() {
    let item = compound("Chain Style Filter", &[fill("0 0 0 1 0 0 0 Xa"), basic("Adobe Zig Zag", "")]);
    let data =
        format!("/KnownStyle :\n(Odd) /Name ,\n/ActiveStyle :\n{}", compound("Chain Style Filter", &[compound("Stack Style Filter", &[item])]));
    assert!(!art_styles(data.as_bytes())["Odd"].plain);
}

#[test]
fn a_hidden_fill_comes_in_hidden() {
    let hidden = fill("0 0 0 1 0 0 0 Xa").replacen("1 /Visible", "0 /Visible", 1);
    let data = format!(
        "/KnownStyle :\n(Hidden) /Name ,\n/ActiveStyle :\n{}",
        compound("Chain Style Filter", &[compound("Stack Style Filter", &[fill("0 0 0 0 1 1 1 Xa"), hidden])])
    );
    let st = &art_styles(data.as_bytes())["Hidden"];
    assert!(st.plain, "{st:?}");
    assert_eq!(st.looks.iter().map(|l| l.visible).collect::<Vec<_>>(), [false, true]);
}
