use proptest::prelude::*;

use super::*;

#[test]
fn important_flag_is_case_insensitive() {
    for s in ["fill: red !IMPORTANT", "fill: red !Important", "fill: red ! important"] {
        assert_eq!(parse_decls(s), [("fill".to_string(), "red".to_string(), true)], "{s}");
    }
    assert_eq!(parse_decls("fill: red !importantx"), [("fill".to_string(), "red !importantx".to_string(), false)]);
}

#[test]
fn semicolons_in_quotes_and_parens_do_not_split() {
    let d = |k: &str, v: &str| (k.to_string(), v.to_string(), false);
    assert_eq!(parse_decls("font-family:'A;B'; fill:red"), [d("font-family", "'A;B'"), d("fill", "red")]);
    assert_eq!(parse_decls("font-family:\"A;B\"; fill:red"), [d("font-family", "\"A;B\""), d("fill", "red")]);
    assert_eq!(
        parse_decls("fill:url(data:image/svg+xml;base64,AAA); stroke:red"),
        [d("fill", "url(data:image/svg+xml;base64,AAA)"), d("stroke", "red")]
    );
    let _ = parse_decls("font-family:'A;B; fill:red");
}

fn with_doc(svg: &str, f: impl FnOnce(&roxmltree::Document, &Styles)) {
    let doc = roxmltree::Document::parse(svg).unwrap();
    let st = Styles::new(&doc);
    f(&doc, &st);
}

fn by_id<'a, 'i>(doc: &'a roxmltree::Document<'i>, id: &str) -> XNode<'a, 'i> {
    doc.descendants().find(|n| n.attribute("id") == Some(id)).unwrap()
}

#[test]
fn selectors_and_precedence() {
    with_doc(
        r#"<svg xmlns="http://www.w3.org/2000/svg"><style>
            /* comment { fill: red } */
            @media print { text { fill: red } }
            @import url(x.css);
            text { fill: #111; font-size: 9px }
            g text { fill: #222 }
            g > text.a { fill: #333 }
            .a.b { stroke: blue }
            #t2 { fill: #444 }
            [data-k=v] { font-weight: bold }
            text:first-child, text ~ text { fill: red }
            .imp { fill: #555 !important }
            </style>
            <text id="t0">x</text>
            <g><text id="t1" class="a b" fill="green">x</text>
               <g><text id="t2" class="a" style="fill:#666">x</text></g>
               <text id="t3" class="a imp" style="fill:#777" data-k="v">x</text></g>
            </svg>"#,
        |doc, st| {
            assert_eq!(st.own(by_id(doc, "t0"), "fill").as_deref(), Some("#111"));
            assert_eq!(st.own(by_id(doc, "t1"), "fill").as_deref(), Some("#333"), "child + class beats descendant");
            assert_eq!(st.own(by_id(doc, "t1"), "stroke").as_deref(), Some("blue"), "multiple classes");
            assert_eq!(st.own(by_id(doc, "t2"), "fill").as_deref(), Some("#666"), "style attribute beats #id");
            assert_eq!(st.own(by_id(doc, "t3"), "fill").as_deref(), Some("#555"), "!important beats style");
            assert_eq!(st.own(by_id(doc, "t3"), "font-weight").as_deref(), Some("bold"));
            assert_eq!(st.own(by_id(doc, "t2"), "stroke"), None, "`.a.b` needs both classes");
            assert_eq!(st.prop(by_id(doc, "t2"), "font-size").as_deref(), Some("9px"));
        },
    );
}

#[test]
fn descendant_matching_does_not_backtrack_combinatorially() {
    // Many nested groups share class `a` and the first rule stacks eight descendant
    // `.a` compounds under a leftmost class that matches nothing: an unmemoized
    // matcher enumerates ancestor combinations for it and effectively never
    // finishes (#1703). With suffix states memoized this returns at once, and the
    // matching descendant and mixed child/descendant rules still apply.
    let depth = 48;
    let mut svg = String::from(
        r#"<svg xmlns="http://www.w3.org/2000/svg"><style>
            .missing .a .a .a .a .a .a .a .a text { fill: #111 }
            .a .a .a text { fill: #222 }
            .a > .a text { stroke: blue }
            </style>"#,
    );
    for _ in 0..depth {
        svg.push_str(r#"<g class="a">"#);
    }
    svg.push_str(r#"<text id="t">x</text>"#);
    for _ in 0..depth {
        svg.push_str("</g>");
    }
    svg.push_str("</svg>");
    with_doc(&svg, |doc, st| {
        let t = by_id(doc, "t");
        assert_eq!(st.own(t, "fill").as_deref(), Some("#222"), "descendant rule matches, the .missing one does not");
        assert_eq!(st.own(t, "stroke").as_deref(), Some("blue"), "mixed child/descendant rule matches");
    });
}

#[test]
fn presentation_attributes_lose_to_rules() {
    with_doc(
        r#"<svg xmlns="http://www.w3.org/2000/svg"><style>#a { fill: blue }</style><text id="a" fill="red" stroke="red">x</text></svg>"#,
        |doc, st| {
            assert_eq!(st.own(by_id(doc, "a"), "fill").as_deref(), Some("blue"));
            assert_eq!(st.own(by_id(doc, "a"), "stroke").as_deref(), Some("red"));
        },
    );
}

/// What [`Styles::declared`] gave before the rules were indexed: every rule tested against `n`.
fn declared_by_every_rule(st: &Styles, n: XNode) -> HashMap<String, String> {
    type Key = (bool, bool, (u32, u32, u32), usize);
    let mut best: HashMap<String, (Key, String)> = HashMap::new();
    let mut put = |name: &str, value: &str, key: Key| {
        if best.get(name).is_none_or(|(k, _)| key >= *k) {
            best.insert(name.to_string(), (key, value.to_string()));
        }
    };
    let mut order = 0;
    for r in st.rules.iter().filter(|r| r.selector.matches(n)) {
        for (k, v, imp) in r.decls.iter() {
            order += 1;
            put(k, v, (*imp, false, r.selector.specificity, order));
        }
    }
    if let Some(s) = n.attribute("style") {
        for (k, v, imp) in parse_decls(s) {
            order += 1;
            put(&k, &v, (imp, true, (0, 0, 0), order));
        }
    }
    best.into_iter().map(|(k, (_, v))| (k, v)).collect()
}

/// Checks that each element of `svg` gets the properties [`declared_by_every_rule`] gives it.
fn assert_same_as_every_rule(svg: &str) {
    let doc = roxmltree::Document::parse(svg).unwrap();
    let st = Styles::new(&doc);
    for n in doc.descendants().filter(|n| n.is_element()) {
        assert_eq!(*st.declared(n), declared_by_every_rule(&st, n), "{n:?} in {svg}");
    }
}

/// Id, class, type, universal and attribute selectors, descendant and child combinators, and
/// rules that override each other give each element what testing every rule gives it.
#[test]
fn indexed_rules_give_what_testing_every_rule_gives() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><style>
            * { stroke: s0 }
            g rect { fill: f1 }
            #a { fill: f2 }
            .a { fill: f3 }
            rect.a { fill: f4 }
            .b.a { stroke: s5 }
            g > .c { fill: f6 }
            svg > g g rect { font-size: 9px }
            [data-k=v] { display: none }
            #a > tspan, text .x { font-size: 8px }
            .c { fill: f8 !important }
            rect { fill: f9 }
            .a { fill: f10 }
            [data-k] { fill: f11 }
            </style>
            <g class="c">
              <rect id="a" class="a"/>
              <rect id="r2" class="b&#9;a&#10;a"/>
              <g class="c x"><rect id="r3" class="c" data-k="v"/><rect id="r4"/></g>
              <circle id="c5" class="a x" data-k="w" style="stroke: s !important"/>
            </g>
            <text id="a" class="a"><tspan id="t6" class="x">t</tspan><tspan id="t7">u</tspan></text>
            <rect id="r8" class="z"/>
            </svg>"#;
    assert_same_as_every_rule(svg);
    with_doc(svg, |doc, st| {
        let own = |id: &str, name: &str| st.own(by_id(doc, id), name);
        assert_eq!(own("a", "fill").as_deref(), Some("f2"), "#a beats rect.a");
        assert_eq!(own("r2", "fill").as_deref(), Some("f4"), "rect.a beats g rect");
        assert_eq!(own("r2", "stroke").as_deref(), Some("s5"), "classes apart by a tab and a line feed, one repeated");
        assert_eq!(own("r3", "fill").as_deref(), Some("f8"), "!important");
        assert_eq!(own("r3", "display").as_deref(), Some("none"));
        assert_eq!(own("r4", "fill").as_deref(), Some("f1"), "g rect beats rect");
        assert_eq!(own("r4", "font-size").as_deref(), Some("9px"));
        assert_eq!(own("c5", "fill").as_deref(), Some("f11"), "of rules alike, the last one");
        assert_eq!(own("c5", "stroke").as_deref(), Some("s"));
        assert_eq!(own("t7", "font-size").as_deref(), Some("8px"));
        assert_eq!(own("r8", "fill").as_deref(), Some("f9"));
    });
}

/// An element's candidates hold each rule once, however often the element repeats a class.
#[test]
fn repeated_classes_give_each_rule_once() {
    with_doc(
        r#"<svg xmlns="http://www.w3.org/2000/svg"><style>.a { fill: x } .b { stroke: y } .a.b { opacity: 0.5 } rect { fill: z }</style><rect id="r" class="a a b a b"/></svg>"#,
        |doc, st| assert_eq!(st.index.candidates(by_id(doc, "r")), [0, 1, 2, 3]),
    );
}

/// A compound selector over a few tags, ids, classes and attributes.
fn arb_compound() -> impl Strategy<Value = String> {
    let tag = prop::sample::select(vec!["", "*", "g", "rect", "text", "tspan"]);
    let id = prop::sample::select(vec!["", "#a", "#b"]);
    let classes = prop::collection::vec(prop::sample::select(vec![".a", ".b", ".c"]), 0..3);
    let attr = prop::sample::select(vec!["", "[data-k]", "[data-k=v]"]);
    (tag, id, classes, attr).prop_map(|(t, i, c, a)| match format!("{t}{i}{}{a}", c.concat()) {
        s if s.is_empty() => "*".to_string(),
        s => s,
    })
}

/// One to three compounds joined by descendant and child combinators.
fn arb_selector() -> impl Strategy<Value = String> {
    (arb_compound(), prop::collection::vec((prop::sample::select(vec![" ", " > ", ">"]), arb_compound()), 0..3))
        .prop_map(|(first, rest)| rest.into_iter().fold(first, |s, (comb, c)| format!("{s}{comb}{c}")))
}

/// A rule of one or two selectors that sets one to three properties, some of them `!important`.
fn arb_rule() -> impl Strategy<Value = String> {
    let decl = (prop::sample::select(vec!["fill", "stroke", "display"]), 0..1000u16, any::<bool>())
        .prop_map(|(name, v, important)| format!("{name}: v{v}{}", if important { " !important" } else { "" }));
    (prop::collection::vec(arb_selector(), 1..3), prop::collection::vec(decl, 1..4))
        .prop_map(|(selectors, decls)| format!("{} {{ {} }}", selectors.join(", "), decls.join("; ")))
}

/// An element that may have an id, classes (some repeated, apart by spaces, tabs and line feeds)
/// and a `data-k` or `style` attribute, with up to four children on each of the `depth` levels
/// below it.
fn arb_element(depth: u32) -> BoxedStrategy<String> {
    let tag = prop::sample::select(vec!["g", "rect", "text", "tspan"]);
    let id = prop::sample::select(vec!["", r#" id="a""#, r#" id="b""#, r#" id="c""#]);
    let classes = prop::collection::vec((prop::sample::select(vec![" ", "&#9;", "&#10;"]), prop::sample::select(vec!["a", "b", "c", "x"])), 0..4)
        .prop_map(|v| match v.iter().map(|(gap, c)| format!("{gap}{c}")).collect::<String>() {
            s if s.is_empty() => s,
            s => format!(r#" class="{s}""#),
        });
    let more = prop::sample::select(vec!["", r#" data-k="v""#, r#" data-k="w""#, r#" style="fill: s""#, r#" style="stroke: s !important""#]);
    let children = if depth == 0 { Just(vec![]).boxed() } else { prop::collection::vec(arb_element(depth - 1), 0..5).boxed() };
    (tag, id, classes, more, children).prop_map(|(t, i, c, m, ch)| format!("<{t}{i}{c}{m}>{}</{t}>", ch.concat())).boxed()
}

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

    /// Random style sheets give each element of a random document what testing every rule
    /// gives it.
    #[test]
    fn random_sheets_give_what_testing_every_rule_gives(rules in prop::collection::vec(arb_rule(), 0..24), body in prop::collection::vec(arb_element(3), 1..6)) {
        assert_same_as_every_rule(&format!(r#"<svg xmlns="http://www.w3.org/2000/svg"><style>{}</style>{}</svg>"#, rules.join("\n"), body.concat()));
    }
}

/// 50,000 class rules give 50,000 elements, each with one of the classes, their fill within
/// 20 s.
#[test]
fn styling_50000_elements_with_50000_rules_finishes_within_20_s() {
    const N: usize = 50_000;
    let rules: String = (0..N).map(|i| format!(".c{i} {{ fill: #{i:06x} }}\n")).collect();
    let rects: String = (0..N).map(|i| format!(r#"<rect class="c{i}"/>"#)).collect();
    let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg"><style>{rules}</style>{rects}</svg>"#);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let doc = roxmltree::Document::parse(&svg).unwrap();
        let st = Styles::new(&doc);
        let fills: Vec<Option<String>> = doc.descendants().filter(|n| n.tag_name().name() == "rect").map(|n| st.own(n, "fill")).collect();
        tx.send(fills).ok();
    });
    let fills = rx.recv_timeout(std::time::Duration::from_secs(20)).unwrap_or_else(|_| panic!("styling {N} elements did not finish within 20 s"));
    assert_eq!(fills.len(), N);
    for (i, fill) in fills.iter().enumerate() {
        assert_eq!(fill.as_deref(), Some(format!("#{i:06x}").as_str()));
    }
}

#[test]
fn the_font_shorthand_sets_its_longhands() {
    let d = parse_decls("fill: red; font: italic bold 30px 'DejaVu Sans' ; font-weight: 300 !important; font: nonsense");
    let get = |k: &str| d.iter().filter(|(n, ..)| n == k).map(|(_, v, i)| (v.as_str(), *i)).collect::<Vec<_>>();
    assert_eq!(get("font-style"), [("italic", false)]);
    assert_eq!(get("font-weight"), [("bold", false), ("300", true)]);
    assert_eq!(get("font-size"), [("30px", false)]);
    assert_eq!(get("font-family"), [("'DejaVu Sans'", false)]);
    assert_eq!(get("font-variant"), [("normal", false)]);
    assert_eq!(get("fill"), [("red", false)]);
    assert!(get("font").is_empty(), "the shorthand itself is not kept");
}

#[test]
fn unsupported_selectors_are_dropped() {
    for s in ["a + b", "a ~ b", "a:hover", "svg|text", "> a", "a >", "", "a:not(b c)", "a:not(.x", "a:not(b > c)", "a:not(:hover)"] {
        assert!(Selector::parse(s).is_none(), "{s}");
    }
    let s = Selector::parse("g  >  text.x#y").unwrap();
    assert_eq!(s.specificity, (1, 1, 2));
    assert_eq!(s.parts.len(), 2);
    assert!(s.parts[0].1, "text must be a child of g");
}

/// #1557: `:not()` of a list of compound selectors; it counts as its most specific argument.
#[test]
fn not_selectors() {
    assert_eq!(Selector::parse("rect:not(.keep)").unwrap().specificity, (0, 1, 1));
    assert_eq!(Selector::parse("g :not(#a, .b.c, text)").unwrap().specificity, (1, 0, 1));
    assert_eq!(Selector::parse(":not(rect):not([x])").unwrap().specificity, (0, 1, 1));
    // Nesting is bounded: a hostile sheet can't recurse without end.
    let nested = |n: usize| format!("{}.x{}", ":not(".repeat(n), ")".repeat(n));
    assert!(Selector::parse(&nested(MAX_NOT_NESTING)).is_some());
    assert!(Selector::parse(&nested(MAX_NOT_NESTING + 1)).is_none());
    assert!(Selector::parse(&nested(200_000)).is_none());
    with_doc(
        r#"<svg xmlns="http://www.w3.org/2000/svg"><style>
            rect:not(.keep) { fill: green }
            g > :not(rect, #t1) { stroke: blue }
            :not(:not(.both)) { opacity: 0.5 }
            .keep { fill: red }
            </style>
            <rect id="r0" fill="red"/><rect id="r1" class="keep"/>
            <g><text id="t0">x</text><text id="t1">x</text><rect id="r2" class="both"/></g>
            </svg>"#,
        |doc, st| {
            assert_eq!(st.own(by_id(doc, "r0"), "fill").as_deref(), Some("green"));
            assert_eq!(st.own(by_id(doc, "r1"), "fill").as_deref(), Some("red"));
            assert_eq!(st.own(by_id(doc, "t0"), "stroke").as_deref(), Some("blue"));
            assert_eq!(st.own(by_id(doc, "t1"), "stroke"), None);
            assert_eq!(st.own(by_id(doc, "r2"), "stroke"), None);
            assert_eq!(st.own(by_id(doc, "r2"), "opacity").as_deref(), Some("0.5"));
            assert_eq!(st.own(by_id(doc, "r0"), "opacity"), None);
            assert!(st.has_not_rules());
            assert_eq!(st.set_by_not_rules(by_id(doc, "r0")), [("fill".to_string(), "green".to_string())]);
            assert!(st.set_by_not_rules(by_id(doc, "r1")).is_empty());
        },
    );
}
