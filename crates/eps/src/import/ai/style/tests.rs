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

#[test]
fn a_drop_shadow_offset_down_points_down() {
    // GUARDS: the document has y down; flipping `vert` like an art-space y would turn the dialog's
    // default shadow (7 pt right, 7 pt down) upwards.
    let data = b"/KnownStyle :\n(Shade) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
            /BasicFilter :\n(Adobe Drop Shadow) 1 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n7 /Real (horz) ,\n7 /Real (vert) ,\n; /Dict ;\n /Part ,\n;\n";
    let styles = art_styles(data);
    let [shadow] = styles["Shade"].effects.as_slice() else { panic!("{:?}", styles["Shade"]) };
    assert_eq!((shadow.params["x"].as_f64(), shadow.params["y"].as_f64()), (Some(7.0), Some(7.0)));
}

/// The style `Shade`: a Drop Shadow whose dictionary holds `keys`.
fn shadow_style(keys: &str) -> ArtStyle {
    let data = format!(
        "/KnownStyle :\n(Shade) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
            /BasicFilter :\n(Adobe Drop Shadow) 1 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n{keys}; /Dict ;\n /Part ,\n;\n"
    );
    art_styles(data.as_bytes()).remove("Shade").unwrap_or_default()
}

#[test]
fn a_drop_shadow_with_darkness_reads_its_darkness() {
    // GUARDS: a `csrc` other than 0 made the style not plain, and the object came in as its drawn look.
    let style = shadow_style("1 /Int (blnd) ,\n0.75 /Real (opac) ,\n5 /Real (blur) ,\n1 /Int (csrc) ,\n50 /Real (dark) ,\n");
    assert!(style.plain, "{style:?}");
    let [shadow] = style.effects.as_slice() else { panic!("{style:?}") };
    let p = &shadow.params;
    assert_eq!(
        (shadow.id.as_str(), &p["shade"], &p["darkness"], &p["opacity"], &p["blur"]),
        ("stylize.dropShadow", &json!("darkness"), &json!(50.0), &json!(75.0), &json!(10.0))
    );
}

#[test]
fn a_drop_shadow_darkness_is_100_percent_when_missing_and_kept_within_0_to_100() {
    for (dark, want) in [("", 100.0), ("(fifty) /Real (dark) ,\n", 100.0), ("250 /Real (dark) ,\n", 100.0), ("-5 /Real (dark) ,\n", 0.0)] {
        let style = shadow_style(&format!("1 /Int (csrc) ,\n{dark}"));
        assert!(style.plain, "{dark:?}: {style:?}");
        let [shadow] = style.effects.as_slice() else { panic!("{dark:?}: {style:?}") };
        assert_eq!((&shadow.params["shade"], &shadow.params["darkness"]), (&json!("darkness"), &json!(want)), "{dark:?}");
    }
}

#[test]
fn a_drop_shadow_with_color_keeps_its_params() {
    // GUARDS: a `csrc` of 0, or none, is Color; its shadow has neither `shade` nor `darkness`.
    for csrc in ["", "0 /Int (csrc) ,\n50 /Real (dark) ,\n"] {
        let style = shadow_style(&format!("2 /Int (blnd) ,\n0.5 /Real (opac) ,\n{csrc}"));
        assert!(style.plain, "{csrc:?}: {style:?}");
        let [shadow] = style.effects.as_slice() else { panic!("{csrc:?}: {style:?}") };
        let want = json!({"mode": "Screen", "opacity": 50.0, "blur": 10.0, "x": 7.0, "y": 7.0, "color": vectorcraft_color::Color::BLACK});
        assert_eq!(shadow.params, want, "{csrc:?}");
    }
}

#[test]
fn a_feather_reads_its_radius_from_rad() {
    let data = b"/KnownStyle :\n(Soft) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
            /BasicFilter :\n(Adobe Feather) 1 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n12 /Real (rad) ,\n; /Dict ;\n /Part ,\n;\n";
    let styles = art_styles(data);
    let [feather] = styles["Soft"].effects.as_slice() else { panic!() };
    assert_eq!((feather.id.as_str(), feather.params["radius"].as_f64()), ("stylize.feather", Some(12.0)));
}

#[test]
fn a_feather_falls_back_to_radius_and_defaults_to_five_points() {
    let with_name = b"/KnownStyle :\n(Soft) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
            /BasicFilter :\n(Adobe Feather) 1 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n3 /Real (radius) ,\n; /Dict ;\n /Part ,\n;\n";
    let styles = art_styles(with_name);
    let [feather] = styles["Soft"].effects.as_slice() else { panic!() };
    assert_eq!(feather.params["radius"].as_f64(), Some(3.0));

    let bare = b"/KnownStyle :\n(Soft) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
            /BasicFilter :\n(Adobe Feather) 1 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n; /Dict ;\n /Part ,\n;\n";
    let styles = art_styles(bare);
    let [feather] = styles["Soft"].effects.as_slice() else { panic!() };
    assert_eq!(feather.params["radius"].as_f64(), Some(5.0));
}

#[test]
fn art_styles_clear_operands_beyond_the_cap() {
    let mut data = Vec::from(
        b"/KnownStyle :\n(Soft) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n/BasicFilter :\n(Adobe Feather) 1 0 /Filter ,\n1 /Visible ,\n",
    );
    for _ in 0..80 {
        data.extend_from_slice(b"1 ");
    }
    data.extend_from_slice(b"/Dictionary : /NotRecorded ,\n; /Dict ;\n /Part ,\n;\n");
    let styles = art_styles(&data);
    let [feather] = styles["Soft"].effects.as_slice() else { panic!() };
    assert_eq!(feather.params["radius"].as_f64(), Some(5.0));
}

#[test]
fn art_styles_ignore_filters_beyond_the_cap() {
    const UNIT: &[u8] = b"/BasicFilter :\n(Adobe Feather) 1 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n; /Dict ;\n /Part ,\n";
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
    let item = compound("Chain Style Filter", &[fill("0 0 0 1 0 0 0 Xa"), basic("Adobe Scribble Fill", "")]);
    let data =
        format!("/KnownStyle :\n(Odd) /Name ,\n/ActiveStyle :\n{}", compound("Chain Style Filter", &[compound("Stack Style Filter", &[item])]));
    assert!(!art_styles(data.as_bytes())["Odd"].plain);
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
