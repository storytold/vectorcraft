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
fn a_style_list_keeps_its_strings_in_order() {
    let list = art_style_list(b"(A)\n(B\\)C)\n(D(E))\n/Name 3 x\n(Caf\\303\\251)\n(F");
    let want: [&[u8]; 4] = [b"A", b"B)C", b"D(E)", "Caf\u{e9}".as_bytes()];
    assert_eq!(list, want, "the string the data ends inside is left out");
    assert!(art_style_list(b"").is_empty());
    let unended: [&[u8]; 1] = [b"A"];
    assert_eq!(art_style_list(b"(A)\n(B\\)"), unended, "an escaped `)` doesn't end the string");
    assert_eq!(art_style_list(b"(A)\n(B(C)"), unended, "nor does one that closes a nested pair");
    assert_eq!(art_style_list(format!("({})", "x".repeat(MAX_LISTED_NAME + 1)).as_bytes()), Vec::<Vec<u8>>::new());
    assert_eq!(art_style_list("(x)\n".repeat(MAX_LISTED + 5).as_bytes()).len(), MAX_LISTED);
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

/// The dictionary of an Effect › Pathfinder filter with `Command` `command` and `Mix` `mix`, and
/// the other options at the values the effect reads.
fn pathfinder_keys(command: &str, mix: &str) -> String {
    format!(
        "0 /Bool (RemovePoints) ,\n{mix} /Real (Mix) ,\n1 /Bool (ConvertCustom) ,\n{command} /Int (Command) ,\n1 /Real (TrapMaxTint) ,\n\
         1 /Real (TrapAspect) ,\n0.4 /Real (TrapTint) ,\n0.25 /Real (TrapThickness) ,\n0 /Bool (TrapReverse) ,\n10 /Real (Precision) ,\n\
         (Op) /UnicodeString (DisplayString) ,\n0 /Bool (TrapConvertCustom) ,\n0.05 /Real (TrapTintTolerance) ,\n1 /Bool (ExtractUnpainted) ,\n"
    )
}

/// The style `Group`: the object's chain holding `effects`, a stack whose fill and stroke paint
/// nothing, and the object's transparency.
fn group_style(effects: &[String]) -> ArtStyle {
    let stack = compound(
        "Stack Style Filter",
        &[
            basic("Stroke Style Filter", "/StrokeStyle : 0 1 0 0 0 Xy\n0 J 0 j 1 w 10 M []0 d\n0 XR\n0 0 Xd\n/Def ;\n (StrokeStyle) ,\n"),
            basic("Conduit Filter", ""),
            basic(
                "Fill Style Filter",
                "/FillStyle : 0 1 0 0 0 Xy\n0 J 0 j 1 w 10 M []0 d\n0 XR\n0 0 Xd\n/Def ;\n (FillStyle) ,\n0 /Bool (UseEvenOdd) ,\n",
            ),
        ],
    );
    let mut kids = effects.to_vec();
    kids.extend([stack, blend(0, 1.0)]);
    let data = format!("/KnownStyle :\n(Group) /Name ,\n/ActiveStyle :\n{}", compound("Chain Style Filter", &kids));
    art_styles(data.as_bytes()).remove("Group").unwrap_or_default()
}

fn pathfinder(keys: &str) -> String {
    basic("Adobe Pathfinder", keys)
}

#[test]
fn pathfinder_add_is_read_as_its_effect() {
    // GUARDS: an `Adobe Pathfinder` filter made the style not plain, so a group with it came in as its drawn look.
    let st = group_style(&[pathfinder(&pathfinder_keys("0", "0.5"))]);
    assert!(st.plain && st.simple && st.pathfinder, "{st:?}");
    assert!(st.items.is_empty(), "{st:?}");
    let [e] = st.effects.as_slice() else { panic!("{st:?}") };
    assert_eq!((e.id.as_str(), &e.params, e.visible), ("pathfinder.add", &json!({}), true));
}

#[test]
fn pathfinder_commands_other_than_add_keep_the_drawn_look() {
    for command in 1..=12 {
        let st = group_style(&[pathfinder(&pathfinder_keys(&command.to_string(), "0.5"))]);
        assert!(!st.plain && !st.pathfinder && st.effects.is_empty(), "{command}: {st:?}");
    }
}

fn stroke(paint: &str) -> String {
    basic("Stroke Style Filter", &format!("/StrokeStyle : 0 1 0 0 0 Xy\n0 J 0 j 2 w 10 M []0 d0 XR\n{paint}\n1 0 Xd\n/Def ;\n (StrokeStyle) ,\n"))
}

fn offset(ofst: f64, jntp: i64, mlim: f64) -> String {
    basic("Adobe Offset Path", &format!("{ofst} /Real (ofst) ,\n{mlim} /Real (mlim) ,\n{jntp} /Int (jntp) ,\n"))
}

/// Style `Fx`: `effects` on the whole object over a stack of a fill (with `on_fill` before its
/// fill style, `after_fill` after it) and a stroke (with `on_stroke` and `after_stroke`).
fn path_style(effects: &[String], on_fill: &[String], after_fill: &[String], on_stroke: &[String], after_stroke: &[String]) -> ArtStyle {
    let item = |before: &[String], paint: String, after: &[String]| {
        if before.is_empty() && after.is_empty() {
            return paint;
        }
        compound("Chain Style Filter", &[before.concat(), paint, after.concat()])
    };
    let stack = compound(
        "Stack Style Filter",
        &[
            item(on_stroke, stroke("0 0 0 1 0 0 0 XA"), after_stroke),
            item(on_fill, fill("0 0 0 0 1 0 0 Xa"), after_fill),
            basic("Conduit Filter", ""),
        ],
    );
    let root = compound("Chain Style Filter", &[effects.concat(), stack]);
    let data = format!("/KnownStyle :\n(Fx) /Name ,\n/ActiveStyle :\n{root}");
    art_styles(data.as_bytes()).remove("Fx").unwrap()
}

fn ids(effects: &[Effect]) -> Vec<&str> {
    effects.iter().map(|e| e.id.as_str()).collect()
}

#[test]
fn offset_path_on_the_object_is_read_with_its_settings() {
    // GUARDS: an object with Offset Path kept its drawn look, and the effect was read only on type.
    let st = path_style(&[offset(4.0, 1, 6.0)], &[], &[], &[], &[]);
    assert!(st.plain && st.simple, "{st:?}");
    let [e] = st.effects.as_slice() else { panic!("{:?}", st.effects) };
    assert_eq!((e.id.as_str(), &e.params), ("path.offsetPath", &json!({"offset": 4.0, "joins": "bevel", "miterLimit": 6.0})));
    assert!(e.visible);
    for (jntp, joins) in [(0, "round"), (2, "miter")] {
        let st = path_style(&[offset(-5.0, jntp, 7.0)], &[], &[], &[], &[]);
        assert_eq!((&st.effects[0].params["joins"], &st.effects[0].params["offset"]), (&json!(joins), &json!(-5.0)));
    }
}

#[test]
fn a_hidden_pathfinder_effect_comes_in_hidden() {
    let st = group_style(&[pathfinder(&pathfinder_keys("0", "0.5")).replacen("1 /Visible", "0 /Visible", 1)]);
    assert!(st.simple, "{st:?}");
    let [e] = st.effects.as_slice() else { panic!("{st:?}") };
    assert_eq!((e.id.as_str(), e.visible), ("pathfinder.add", false));
}

#[test]
fn pathfinder_effects_keep_their_order_among_the_objects_effects() {
    let offset = basic("Adobe Offset Path", "3 /Real (ofst) ,\n0 /Int (jntp) ,\n4 /Real (mlim) ,\n");
    let st = group_style(&[pathfinder(&pathfinder_keys("0", "0.5")), offset.clone()]);
    assert!(st.plain, "{st:?}");
    assert_eq!(ids(&st.effects), ["pathfinder.add", "path.offsetPath"]);
    // GUARDS: an effect before the Pathfinder effect was read too, and the group drew the Pathfinder
    // effect first, which isn't what the file draws.
    let st = group_style(&[offset, pathfinder(&pathfinder_keys("0", "0.5"))]);
    assert!(!st.plain, "{st:?}");
}

#[test]
fn pathfinder_settings_this_does_not_read_keep_the_drawn_look() {
    let add = pathfinder_keys("0", "0.5");
    let cases = [
        ("no Command", add.replace("0 /Int (Command) ,\n", "")),
        ("Trap", add.replace("0 /Int (Command)", "12 /Int (Command)")),
        ("unknown Command", add.replace("0 /Int (Command)", "40 /Int (Command)")),
        ("negative Command", add.replace("0 /Int (Command)", "-1 /Int (Command)")),
        ("fractional Command", add.replace("0 /Int (Command)", "2.5 /Int (Command)")),
        ("huge Command", add.replace("0 /Int (Command)", "1e999 /Int (Command)")),
        ("Command as a real", add.replace("0 /Int (Command)", "0 /Real (Command)")),
        ("Command without a number", add.replace("0 /Int (Command)", "(zero) /Int (Command)")),
        // A number before the string isn't its value.
        ("Command with a string after a number", add.replace("0 /Int (Command)", "3 (zero) /Int (Command)")),
        ("Command twice", add.clone() + "1 /Int (Command) ,\n"),
        // GUARDS: the last of a repeated key was read, so a Trap command followed by Add was read as Add.
        ("Command twice, Add last", add.replace("0 /Int (Command)", "12 /Int (Command) ,\n0 /Int (Command)")),
        ("Command twice, both Add", add.replace("0 /Int (Command)", "0 /Int (Command) ,\n0 /Int (Command)")),
        ("an option twice, the read value last", add.replace("10 /Real (Precision)", "5 /Real (Precision) ,\n10 /Real (Precision)")),
        ("unknown key", add.clone() + "1 /Int (Extra) ,\n"),
        ("unknown string", add.clone() + "(x) /String (Extra) ,\n"),
        ("an array", add.clone() + "/Array :\n1 /Real ,\n; (Extra) ,\n"),
        ("many strings", add.clone() + &(0..40).map(|i| format!("(x) /String (DisplayString{i}) ,\n")).collect::<String>()),
        ("missing option", add.replace("0.25 /Real (TrapThickness) ,\n", "")),
        ("missing Mix", add.replace("0.5 /Real (Mix) ,\n", "")),
        ("option of the wrong type", add.replace("10 /Real (Precision)", "10 /Int (Precision)")),
        ("other Precision", add.replace("10 /Real (Precision)", "5 /Real (Precision)")),
        ("huge Precision", add.replace("10 /Real (Precision)", "1e999 /Real (Precision)")),
        ("Remove Redundant Points", add.replace("0 /Bool (RemovePoints)", "1 /Bool (RemovePoints)")),
        ("unpainted art kept", add.replace("1 /Bool (ExtractUnpainted)", "0 /Bool (ExtractUnpainted)")),
        ("other trap tint", add.replace("0.4 /Real (TrapTint)", "0.25 /Real (TrapTint)")),
        ("Mix on Add", add.replace("0.5 /Real (Mix)", "0.3 /Real (Mix)")),
        ("Mix as a bool", add.replace("0.5 /Real (Mix)", "1 /Bool (Mix)")),
    ];
    for (case, keys) in cases {
        let st = group_style(&[pathfinder(&keys)]);
        assert!(!st.plain && !st.pathfinder && st.effects.is_empty(), "{case}: {st:?}");
    }
}

#[test]
fn a_pathfinder_effect_outside_the_objects_chain_keeps_the_drawn_look() {
    let pf = pathfinder(&pathfinder_keys("0", "0.5"));
    // On a fill of its own.
    let item = compound("Chain Style Filter", &[pf.clone(), fill("0 0 0 1 0 0 0 Xa")]);
    let data =
        format!("/KnownStyle :\n(Item) /Name ,\n/ActiveStyle :\n{}", compound("Chain Style Filter", &[compound("Stack Style Filter", &[item])]));
    let st = &art_styles(data.as_bytes())["Item"];
    assert!(!st.plain, "{st:?}");
    // In a style that isn't an object's chain.
    let data = format!("/KnownStyle :\n(Bare) /Name ,\n/ActiveStyle :\n{pf}");
    let st = &art_styles(data.as_bytes())["Bare"];
    assert!(!st.plain && st.effects.is_empty(), "{st:?}");
}

#[test]
fn a_pathfinder_effect_with_fills_of_its_own_keeps_the_drawn_look() {
    let stack = compound("Stack Style Filter", &[fill("0 0 0 1 0 0 0 Xa"), basic("Conduit Filter", "")]);
    let data = format!(
        "/KnownStyle :\n(Filled) /Name ,\n/ActiveStyle :\n{}",
        compound("Chain Style Filter", &[pathfinder(&pathfinder_keys("0", "0.5")), stack])
    );
    let st = &art_styles(data.as_bytes())["Filled"];
    assert!(st.pathfinder && !st.simple, "{st:?}");
}

#[test]
fn outline_object_and_outline_stroke_are_read_with_no_settings() {
    // GUARDS: `Adobe Outline Type` and `Adobe Outline Stroke` weren't read, so the style kept its drawn look.
    let outline = basic("Adobe Outline Type", "");
    let outline_stroke = basic("Adobe Outline Stroke", "");
    let st = path_style(&[outline.clone(), outline_stroke.clone()], &[], &[], &[], &[]);
    assert!(st.plain && st.simple, "{st:?}");
    assert_eq!(ids(&st.effects), ["path.outlineObject", "path.outlineStroke"]);
    // No `width`: the stroke gives it.
    assert!(st.effects.iter().all(|e| e.params == json!({})), "{:?}", st.effects);
    // On a fill and on a stroke: Outline Object before the item's own style, Outline Stroke after it.
    let (before, after) = (std::slice::from_ref(&outline), std::slice::from_ref(&outline_stroke));
    let st = path_style(&[], before, after, before, after);
    assert!(st.plain && !st.simple, "{st:?}");
    // Bottom first: the fill, then the stroke.
    assert!(matches!(st.items.as_slice(), [StyleItem::Fill(_), StyleItem::Stroke { .. }]), "{:?}", st.items);
    for look in &st.looks {
        assert_eq!(ids(&look.effects), ["path.outlineObject", "path.outlineStroke"]);
    }
}

#[test]
fn path_effects_keep_their_order_and_visibility() {
    let hidden = offset(3.0, 0, 4.0).replacen("1 /Visible", "0 /Visible", 1);
    let st = path_style(&[hidden, offset(6.0, 1, 4.0)], &[], &[], &[], &[]);
    assert!(st.plain && st.simple, "{st:?}");
    let offsets: Vec<_> = st.effects.iter().map(|e| (e.params["offset"].as_f64(), e.visible)).collect();
    assert_eq!(offsets, [(Some(3.0), false), (Some(6.0), true)]);
    // On a fill and a stroke, with another effect after it.
    let transform = basic("Adobe Transform", "2 /Int (numCopies) ,\n");
    let after = std::slice::from_ref(&transform);
    let st = path_style(&[], &[offset(2.0, 2, 4.0)], after, &[offset(-1.0, 0, 4.0)], after);
    assert!(st.plain, "{st:?}");
    assert_eq!(ids(&st.looks[0].effects), ["path.offsetPath", "distort.transform"]);
    assert_eq!(ids(&st.looks[1].effects), ["path.offsetPath", "distort.transform"]);
    assert_eq!((st.looks[0].effects[0].params["offset"].as_f64(), st.looks[1].effects[0].params["offset"].as_f64()), (Some(2.0), Some(-1.0)));
}

#[test]
fn offset_path_with_settings_not_read_keeps_the_drawn_look() {
    // GUARDS: a missing setting was read as 10 pt, miter or 4, and an out-of-range one was clamped.
    let complete = "4 /Real (ofst) ,\n1 /Int (jntp) ,\n6 /Real (mlim) ,\n";
    let bodies = [
        String::new(),
        "4 /Real (ofst) ,\n".into(),
        "1 /Int (jntp) ,\n".into(),
        "4 /Real (ofst) ,\n1 /Int (jntp) ,\n".into(),
        format!("{complete}1 /Int (later) ,\n"),
        format!("{complete}(x) /UnicodeString (DisplayString) ,\n"),
        complete.replace("/Int (jntp)", "/Real (jntp)"),
        complete.replace("4 /Real (ofst)", "(4) /Real (ofst)"),
        complete.replace("4 /Real (ofst)", "1e999 /Real (ofst)"),
        complete.replace("4 /Real (ofst)", "100001 /Real (ofst)"),
        complete.replace("6 /Real (mlim)", "0.5 /Real (mlim)"),
        complete.replace("6 /Real (mlim)", "501 /Real (mlim)"),
        complete.replace("1 /Int (jntp)", "3 /Int (jntp)"),
        complete.replace("1 /Int (jntp)", "-1 /Int (jntp)"),
        complete.replace("1 /Int (jntp)", "0.5 /Int (jntp)"),
    ];
    for body in &bodies {
        let filter = basic("Adobe Offset Path", body);
        let one = std::slice::from_ref(&filter);
        for st in [path_style(one, &[], &[], &[], &[]), path_style(&[], one, &[], &[], &[])] {
            assert!(!st.plain, "{body:?}: {st:?}");
        }
    }
    assert!(path_style(&[basic("Adobe Offset Path", complete)], &[], &[], &[], &[]).plain);
    // The outline effects have no settings.
    for name in ["Adobe Outline Type", "Adobe Outline Stroke"] {
        assert!(!path_style(&[basic(name, "1 /Real (width) ,\n")], &[], &[], &[], &[]).plain, "{name}");
        assert!(!path_style(&[basic(name, "/Array : 1 /Real , ; (width) ,\n")], &[], &[], &[], &[]).plain, "{name}");
    }
}

/// The style `Fx`: effect `name` on the whole object, its dictionary holding `keys`.
fn distort_effect_style(name: &str, keys: &str) -> ArtStyle {
    let data = format!("/KnownStyle :\n(Fx) /Name ,\n/ActiveStyle :\n{}", compound("Chain Style Filter", &[basic(name, keys)]));
    art_styles(data.as_bytes()).remove("Fx").unwrap_or_default()
}

/// The params of effect `name` with `keys`, `None` when the style keeps the drawn look.
fn read_effect(name: &str, keys: &str) -> Option<serde_json::Value> {
    let st = distort_effect_style(name, keys);
    if !st.plain {
        return None;
    }
    let [e] = st.effects.as_slice() else { panic!("{st:?}") };
    Some(e.params.clone())
}

/// A filter named `name` whose dictionary holds `entries`.
fn dict(name: &str, entries: &[(&str, Kind, f64)]) -> Filter {
    let kinds = entries.iter().map(|(k, kind, _)| (k.to_string(), *kind)).collect();
    let params = entries.iter().map(|(k, _, v)| (k.to_string(), *v)).collect();
    Filter { name: name.into(), kinds, params, ..Filter::default() }
}

fn close(a: &serde_json::Value, want: f64) -> bool {
    a.as_f64().is_some_and(|a| (a - want).abs() < 1e-6)
}

/// Every switch of the Transform effect, as files write them.
const SWITCHES: &str = "0 /Bool (scaleLines) ,\n0 /Bool (transformPatterns) ,\n";

#[test]
fn a_transform_reads_its_factors_and_radians() {
    // GUARDS: the percent and degree entries (what the dialog shows) were read, and the options were not.
    let p = read_effect(
        "Adobe Transform",
        "150 /Real (scaleH_Percent) ,\n1.4 /Real (scaleH_Factor) ,\n80 /Real (scaleV_Percent) ,\n1.2 /Real (scaleV_Factor) ,\n\
         30 /Real (rotate_Degrees) ,\n0.471238898 /Real (rotate_Radians) ,\n-8 /Real (moveV_Pts) ,\n3 /Real (moveH_Pts) ,\n\
         2 /Int (numCopies) ,\n0 /Int (pinPoint) ,\n1 /Bool (reflectX) ,\n0 /Bool (transformObjects) ,\n1 /Bool (transformPatterns) ,\n\
         1 /Bool (scaleLines) ,\n0 /Bool (randomize) ,\n",
    )
    .unwrap();
    assert!(close(&p["scaleH"], 140.0) && close(&p["scaleV"], 120.0) && close(&p["rotate"], 27.0), "{p}");
    assert_eq!((&p["moveH"], &p["moveV"], &p["copies"], &p["reference"]), (&json!(3.0), &json!(8.0), &json!(2), &json!(0)));
    assert_eq!((&p["reflectX"], &p["reflectY"]), (&json!(true), &json!(false)));
    assert_eq!((&p["objects"], &p["patterns"], &p["strokes"]), (&json!(false), &json!(true), &json!(true)));
}

#[test]
fn a_transforms_percent_and_degrees_alone_change_nothing() {
    let p = read_effect("Adobe Transform", &format!("150 /Real (scaleH_Percent) ,\n30 /Real (rotate_Degrees) ,\n{SWITCHES}")).unwrap();
    assert_eq!((&p["scaleH"], &p["scaleV"], &p["rotate"]), (&json!(100.0), &json!(100.0), &json!(0.0)));
}

#[test]
fn a_transforms_reference_point_and_move_up() {
    for pin in 0..9 {
        let p = read_effect("Adobe Transform", &format!("0.5 /Real (scaleH_Factor) ,\n{pin} /Int (pinPoint) ,\n{SWITCHES}")).unwrap();
        assert_eq!(p["reference"], json!(pin));
    }
    // A move up the page in the file is a move up in the document, whose y runs down.
    let p = read_effect("Adobe Transform", &format!("10 /Real (moveV_Pts) ,\n{SWITCHES}")).unwrap();
    assert_eq!(p["moveV"], json!(-10.0));
    let p = read_effect("Adobe Transform", &format!("4 /Int (pinPoint) ,\n{SWITCHES}")).unwrap();
    assert_eq!((&p["objects"], &p["copies"], &p["scaleH"]), (&json!(true), &json!(0), &json!(100.0)));
}

#[test]
fn a_transform_without_the_options_its_change_depends_on_keeps_the_drawn_look() {
    // Missing Scale Strokes & Effects or Transform Patterns are the app's settings, not the file's.
    let half = "0.5 /Real (scaleH_Factor) ,\n0.5 /Real (scaleV_Factor) ,\n";
    assert_eq!(read_effect("Adobe Transform", &format!("{half}0 /Bool (transformPatterns) ,\n")), None);
    assert_eq!(read_effect("Adobe Transform", &format!("{half}0 /Bool (scaleLines) ,\n")), None);
    // A move scales nothing; with Transform Patterns given, it reads.
    let p = read_effect("Adobe Transform", "5 /Real (moveH_Pts) ,\n0 /Bool (transformPatterns) ,\n").unwrap();
    assert_eq!(p["strokes"], json!(false));
    // On a fill's own Transform, only that fill's paint counts: a solid fill moves the same either way.
    let moved = basic("Adobe Transform", "5 /Real (moveH_Pts) ,\n");
    let item = |paint: &str| {
        let item = compound("Chain Style Filter", &[fill(paint), moved.clone()]);
        let data =
            format!("/KnownStyle :\n(Fx) /Name ,\n/ActiveStyle :\n{}", compound("Chain Style Filter", &[compound("Stack Style Filter", &[item])]));
        art_styles(data.as_bytes()).remove("Fx").unwrap_or_default().plain
    };
    assert!(item("0 0 0 1 0 0 0 Xa"));
    assert!(!item("(Dots) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p"));
}

#[test]
fn a_random_transform_keeps_the_drawn_look() {
    // The file holds no seed for its random share.
    assert_eq!(read_effect("Adobe Transform", &format!("1 /Bool (randomize) ,\n{SWITCHES}")), None);
}

/// Free Distort's dictionary: the source corners `src` and the destination `dst`, top left, top
/// right, bottom left, bottom right, y up.
fn corners(src: [[f64; 2]; 4], dst: [[f64; 2]; 4]) -> String {
    let mut out = String::new();
    for (which, c) in [("src", src), ("dst", dst)] {
        for (i, [h, v]) in c.into_iter().enumerate() {
            out += &format!("{h} /Real ({which}{i}h) ,\n{v} /Real ({which}{i}v) ,\n");
        }
    }
    out
}

fn unit_corners(p: &serde_json::Value) -> Vec<[f64; 2]> {
    p["corners"].as_array().unwrap().iter().map(|c| [c[0].as_f64().unwrap(), c[1].as_f64().unwrap()]).collect()
}

fn corners_near(a: &[[f64; 2]], b: &[[f64; 2]]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9)
}

#[test]
fn free_distort_measures_its_corners_in_its_own_frame() {
    // GUARDS: Free Distort wasn't read, and type with it came in as its drawn look.
    let dst = [[180.0, 310.0], [320.0, 300.0], [210.0, 240.0], [250.0, 260.0]];
    // A frame wider than the object (its stroke's): the corners are fractions of that frame.
    let p = read_effect("Adobe Free Distort", &corners([[195.0, 305.0], [305.0, 305.0], [195.0, 245.0], [305.0, 245.0]], dst)).unwrap();
    let want = [[-15.0 / 110.0, -5.0 / 60.0], [125.0 / 110.0, 5.0 / 60.0], [0.5, 0.75], [15.0 / 110.0, 65.0 / 60.0]];
    assert!(corners_near(&unit_corners(&p), &want), "{p}");
    // A unit frame: the same corners measured in it, far outside the unit box.
    let p = read_effect("Adobe Free Distort", &corners([[0.0, 0.0], [1.0, 0.0], [0.0, -1.0], [1.0, -1.0]], dst)).unwrap();
    assert!(corners_near(&unit_corners(&p), &[[180.0, -310.0], [320.0, -300.0], [250.0, -260.0], [210.0, -240.0]]), "{p}");
    // No corners: no change.
    let p = read_effect("Adobe Free Distort", "").unwrap();
    assert!(corners_near(&unit_corners(&p), &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]), "{p}");
}

#[test]
fn free_distort_without_every_corner_or_an_upright_frame_keeps_the_drawn_look() {
    let src = [[0.0, 0.0], [1.0, 0.0], [0.0, -1.0], [1.0, -1.0]];
    let dst = [[0.1, 0.0], [1.0, 0.1], [0.0, -1.0], [1.0, -1.2]];
    let all = corners(src, dst);
    // The destination alone, one entry missing, a frame that is turned or of no height.
    let dst_only: String = all.lines().filter(|l| l.contains("(dst")).map(|l| format!("{l}\n")).collect();
    let short = all.replace("-1.2 /Real (dst3v) ,\n", "");
    for keys in [
        dst_only,
        short,
        corners([[0.0, 0.0], [1.0, 0.5], [0.0, -1.0], [1.0, -1.0]], dst),
        corners([[0.0, 0.0], [1.0, 0.0], [0.0, 0.0], [1.0, 0.0]], dst),
        // A frame too wide for a number.
        corners([[-1e308, 0.0], [1e308, 0.0], [-1e308, -1.0], [1e308, -1.0]], dst),
    ] {
        assert_eq!(read_effect("Adobe Free Distort", &keys), None, "{keys}");
    }
}

#[test]
fn pucker_and_twist_keep_their_sense() {
    // Negative is Pucker; a positive angle turns clockwise in the file and in the catalog.
    assert_eq!(read_effect("Adobe Punk and Bloat", "-35 /Real (d_factor) ,\n"), Some(json!({"amount": -35.0})));
    assert_eq!(read_effect("Adobe Punk and Bloat", ""), Some(json!({"amount": 0.0})));
    assert_eq!(read_effect("Adobe Twirl", "45 /Real (angle) ,\n"), Some(json!({"angle": 45.0})));
    assert_eq!(read_effect("Adobe Twirl", ""), Some(json!({"angle": 0.0})));
    assert_eq!(read_effect("Adobe Punk and Bloat", "250 /Real (d_factor) ,\n"), None);
    assert_eq!(read_effect("Adobe Twirl", "4000 /Real (angle) ,\n"), None);
}

#[test]
fn roughen_keeps_the_drawn_look() {
    // Its outline is random and the file holds no seed; at a size of 0 the effect still redraws the
    // path's segments.
    assert_eq!(read_effect("Adobe Roughen", "30 /Real (size) ,\n"), None);
    assert_eq!(read_effect("Adobe Roughen", ""), None);
}

#[test]
fn tweak_picks_its_amounts_by_its_mode_and_moves_every_point_unless_told() {
    let amounts = [("horz", Kind::Real, 30.0), ("vert", Kind::Real, 20.0), ("ahor", Kind::Real, 2.0), ("aver", Kind::Real, 3.0)];
    let mut absolute = amounts.to_vec();
    absolute.push(("absoluteness", Kind::Real, 1.0));
    let want = |h: f64, v: f64, relative: bool| json!({"h": h, "v": v, "relative": relative, "anchors": true, "in": true, "out": true});
    assert_eq!(tweak(&dict("Adobe Scribble and Tweak", &amounts)), Some(want(30.0, 20.0, true)));
    assert_eq!(tweak(&dict("Adobe Scribble and Tweak", &absolute)), Some(want(2.0, 3.0, false)));
    let off = tweak(&dict("Adobe Scribble and Tweak", &[("in", Kind::Bool, 0.0)])).unwrap();
    assert_eq!((&off["anchors"], &off["in"], &off["out"]), (&json!(true), &json!(false), &json!(true)));
    // Random too: it reads only when it moves nothing.
    assert_eq!(read_effect("Adobe Scribble and Tweak", "30 /Real (horz) ,\n"), None);
    assert!(read_effect("Adobe Scribble and Tweak", "").is_some());
    let still = "30 /Real (horz) ,\n0 /Bool (anch) ,\n0 /Bool (in) ,\n0 /Bool (out) ,\n";
    assert!(read_effect("Adobe Scribble and Tweak", still).is_some());
}

#[test]
fn zig_zag_is_absolute_with_four_ridges_and_corners_unless_told() {
    let p = read_effect("Adobe Zigzag", "30 /Real (relAmount) ,\n2 /Real (amount) ,\n").unwrap();
    assert_eq!(p, json!({"size": 2.0, "relative": false, "ridges": 4, "points": "corner"}));
    let p = read_effect(
        "Adobe Zigzag",
        "30 /Real (relAmount) ,\n2 /Real (amount) ,\n0 /Real (absoluteness) ,\n6 /Real (ridges) ,\n1 /Real (roundness) ,\n",
    )
    .unwrap();
    assert_eq!(p, json!({"size": 30.0, "relative": true, "ridges": 6, "points": "smooth"}));
    assert_eq!(read_effect("Adobe Zigzag", "2.5 /Real (ridges) ,\n"), None);
}

#[test]
fn a_distort_entry_this_cant_read_keeps_the_drawn_look_and_unknown_ones_are_ignored() {
    // Not a number, the wrong kind, a switch other than 0 or 1, a whole number that isn't.
    for (name, keys) in [
        ("Adobe Twirl", "NaN /Real (angle) ,\n"),
        ("Adobe Twirl", "45 /Int (angle) ,\n"),
        ("Adobe Zigzag", "2 /Real (absoluteness) ,\n"),
        ("Adobe Transform", "4 /Real (pinPoint) ,\n0 /Bool (scaleLines) ,\n0 /Bool (transformPatterns) ,\n"),
        ("Adobe Transform", "2 /Bool (reflectX) ,\n0 /Bool (scaleLines) ,\n0 /Bool (transformPatterns) ,\n"),
        ("Adobe Transform", "1e999 /Real (scaleH_Percent) ,\n0 /Bool (scaleLines) ,\n0 /Bool (transformPatterns) ,\n"),
        ("Adobe Transform", "1001 /Int (numCopies) ,\n0 /Bool (scaleLines) ,\n0 /Bool (transformPatterns) ,\n"),
        // In degrees, too large for a number.
        ("Adobe Transform", "1e308 /Real (rotate_Radians) ,\n0 /Bool (scaleLines) ,\n0 /Bool (transformPatterns) ,\n"),
        ("Adobe Free Distort", "(x) /Real (src0h) ,\n"),
    ] {
        assert_eq!(read_effect(name, keys), None, "{name}: {keys}");
    }
    assert_eq!(read_effect("Adobe Twirl", "37 /Real (angle) ,\n9 /Real (futureKey) ,\n"), Some(json!({"angle": 37.0})));
    // The last of a repeated entry stands.
    assert_eq!(read_effect("Adobe Twirl", "37 /Real (angle) ,\n12 /Real (angle) ,\n"), Some(json!({"angle": 12.0})));
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

/// A Convert to Shape filter whose dictionary holds `keys`.
fn shape(keys: &str) -> String {
    basic("Adobe Shape Effects", &format!("(Shape) /UnicodeString (DisplayString) ,\n{keys}"))
}

/// The style `Boxed`: a fill and a stroke, and `effects` on the whole object after them.
fn shape_style(effects: &[String]) -> ArtStyle {
    let stack = compound(
        "Stack Style Filter",
        &[
            basic("Stroke Style Filter", "/StrokeStyle : 0 R\n0 0 0 1 0 0 0 XA\n0 J 0 j 2 w 10 M []0 d\n0 XR\n1 0 Xd\n/Def ;\n (StrokeStyle) ,\n"),
            fill("0 0 0 0 1 1 1 Xa"),
        ],
    );
    let mut kids = vec![stack];
    kids.extend_from_slice(effects);
    let data = format!("/KnownStyle :\n(Boxed) /Name ,\n/ActiveStyle :\n{}", compound("Chain Style Filter", &kids));
    art_styles(data.as_bytes()).remove("Boxed").unwrap_or_default()
}

/// Every setting of a Relative or Absolute size, with a corner radius.
const SIZES: &str = "150 /Real (AbsWidth) ,\n40 /Real (AbsHeight) ,\n20 /Real (RelWidth) ,\n6 /Real (RelHeight) ,\n15 /Real (CornerRadius) ,\n";

#[test]
fn convert_to_shape_comes_in_live_with_its_settings() {
    // GUARDS: `Adobe Shape Effects` wasn't read, so an object with it kept its drawn look.
    for (keys, id, want) in [
        (
            format!("0 /Int (Shape) ,\n0 /Real (Absolute) ,\n{SIZES}"),
            "convertToShape.rectangle",
            json!({"relative": true, "extraW": 40.0, "extraH": 12.0, "width": 150.0, "height": 40.0}),
        ),
        (
            format!("1 /Int (Shape) ,\n0 /Real (Absolute) ,\n{SIZES}"),
            "convertToShape.roundedRectangle",
            json!({"relative": true, "extraW": 40.0, "extraH": 12.0, "width": 150.0, "height": 40.0, "radius": 15.0}),
        ),
        (
            format!("2 /Int (Shape) ,\n1 /Real (Absolute) ,\n{SIZES}"),
            "convertToShape.ellipse",
            json!({"relative": false, "extraW": 40.0, "extraH": 12.0, "width": 150.0, "height": 40.0}),
        ),
        (
            format!("0 /Int (Shape) ,\n1 /Real (Absolute) ,\n{SIZES}"),
            "convertToShape.rectangle",
            json!({"relative": false, "extraW": 40.0, "extraH": 12.0, "width": 150.0, "height": 40.0}),
        ),
        // A relative size alone, and one that shrinks the shape: each is added to every side.
        (
            "0 /Int (Shape) ,\n0 /Real (Absolute) ,\n20 /Real (RelWidth) ,\n6 /Real (RelHeight) ,\n".to_string(),
            "convertToShape.rectangle",
            json!({"relative": true, "extraW": 40.0, "extraH": 12.0, "width": 36.0, "height": 36.0}),
        ),
        (
            "0 /Int (Shape) ,\n0 /Real (Absolute) ,\n-10 /Real (RelWidth) ,\n-4 /Real (RelHeight) ,\n".to_string(),
            "convertToShape.rectangle",
            json!({"relative": true, "extraW": -20.0, "extraH": -8.0, "width": 36.0, "height": 36.0}),
        ),
        // Missing sizes: 18 pt a side, 36 pt, and a 9 pt radius.
        (
            "0 /Int (Shape) ,\n0 /Real (Absolute) ,\n20 /Real (RelWidth) ,\n".to_string(),
            "convertToShape.rectangle",
            json!({"relative": true, "extraW": 40.0, "extraH": 36.0, "width": 36.0, "height": 36.0}),
        ),
        (
            "1 /Int (Shape) ,\n1 /Real (Absolute) ,\n".to_string(),
            "convertToShape.roundedRectangle",
            json!({"relative": false, "extraW": 36.0, "extraH": 36.0, "width": 36.0, "height": 36.0, "radius": 9.0}),
        ),
    ] {
        let st = shape_style(&[shape(&keys)]);
        assert!(st.plain && st.simple, "{keys}: {st:?}");
        let [e] = st.effects.as_slice() else { panic!("{keys}: {:?}", st.effects) };
        assert_eq!((e.id.as_str(), &e.params, e.visible), (id, &want, true), "{keys}");
        assert_eq!(st.items.len(), 2, "{keys}");
    }
}

#[test]
fn convert_to_shape_keeps_its_place_among_effects_and_its_visibility() {
    let offset = basic("Adobe Offset Path", "2 /Int (jntp) ,\n4 /Real (mlim) ,\n4 /Real (ofst) ,\n");
    let st = shape_style(&[shape(&format!("0 /Int (Shape) ,\n0 /Real (Absolute) ,\n{SIZES}")), offset.clone()]);
    assert!(st.plain, "{st:?}");
    let ids: Vec<_> = st.effects.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["convertToShape.rectangle", "path.offsetPath"]);
    // Offset Path is read on any object too.
    assert!(st.simple);
    // A hidden one stays, hidden, with its settings.
    let hidden = shape(&format!("1 /Int (Shape) ,\n0 /Real (Absolute) ,\n{SIZES}")).replacen("1 /Visible", "0 /Visible", 1);
    let st = shape_style(&[hidden]);
    assert!(st.plain && st.simple, "{st:?}");
    let [e] = st.effects.as_slice() else { panic!("{:?}", st.effects) };
    assert_eq!(
        (e.id.as_str(), e.visible, &e.params["radius"], &e.params["extraW"]),
        ("convertToShape.roundedRectangle", false, &json!(15.0), &json!(40.0))
    );
}

#[test]
fn convert_to_shape_settings_that_arent_read_keep_the_drawn_look() {
    let valid = format!("0 /Int (Shape) ,\n0 /Real (Absolute) ,\n{SIZES}");
    assert!(shape_style(&[shape(&valid)]).plain);
    for keys in [
        // Another shape, a shape that isn't a whole number or an /Int, and a missing one.
        valid.replace("0 /Int (Shape)", "3 /Int (Shape)"),
        valid.replace("0 /Int (Shape)", "0.5 /Int (Shape)"),
        valid.replace("0 /Int (Shape)", "0 /Real (Shape)"),
        valid.replace("0 /Int (Shape) ,\n", ""),
        // A mode other than 0 and 1, or a missing one.
        valid.replace("0 /Real (Absolute)", "2 /Real (Absolute)"),
        valid.replace("0 /Real (Absolute) ,\n", ""),
        valid.replace("0 /Real (Absolute)", "0 /Int (Absolute)"),
        // Sizes that aren't finite numbers, negative absolute sizes and radii, and huge ones.
        valid.replace("20 /Real (RelWidth)", "NaN /Real (RelWidth)"),
        valid.replace("6 /Real (RelHeight)", "1e999 /Real (RelHeight)"),
        valid.replace("6 /Real (RelHeight)", "(x) /Real (RelHeight)"),
        valid.replace("150 /Real (AbsWidth)", "-150 /Real (AbsWidth)"),
        valid.replace("15 /Real (CornerRadius)", "-1 /Real (CornerRadius)"),
        valid.replace("20 /Real (RelWidth)", "1e6 /Real (RelWidth)"),
        valid.replace("40 /Real (AbsHeight)", "40 /Int (AbsHeight)"),
        // A key that isn't read.
        format!("{valid}1 /Real (Future) ,\n"),
        format!("{valid}(x) /String (Future) ,\n"),
        // A relative Ellipse.
        valid.replace("0 /Int (Shape)", "2 /Int (Shape)"),
    ] {
        let st = shape_style(&[shape(&keys)]);
        assert!(!st.plain, "{keys}: {st:?}");
    }
    // On one fill: the drawn look, for type too.
    let item = compound("Chain Style Filter", &[fill("0 0 0 1 0 0 0 Xa"), shape(&valid)]);
    let data =
        format!("/KnownStyle :\n(Odd) /Name ,\n/ActiveStyle :\n{}", compound("Chain Style Filter", &[compound("Stack Style Filter", &[item])]));
    assert!(!art_styles(data.as_bytes())["Odd"].plain);
}
