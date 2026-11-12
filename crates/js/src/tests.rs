use proptest::prelude::*;

use crate::{
    BinaryOp, Expr, ExprId, ForInTarget, FunctionKind, Options, Program, PropertyValue, Stmt, StmtId, SyntaxError, TOO_DEEP, TokenKind, parse,
    tokenize,
};

fn ok(src: &str) -> Program {
    match parse(src, &Options::default()) {
        Ok(p) => p,
        Err(e) => panic!("{src:?} should parse: {e}"),
    }
}

fn err(src: &str) -> SyntaxError {
    match parse(src, &Options::default()) {
        Ok(_) => panic!("{src:?} should not parse"),
        Err(e) => e,
    }
}

fn strict_err(src: &str) -> SyntaxError {
    let full = format!("'use strict';\n{src}");
    match parse(&full, &Options::default()) {
        Ok(_) => panic!("{src:?} should not parse in strict mode code"),
        Err(e) => e,
    }
}

fn with_preprocessor() -> Options {
    Options { preprocessor_lines: true, ..Options::default() }
}

/// The expression of the program's only (or first) statement.
fn first_expr(p: &Program) -> ExprId {
    match p.ast.stmt(p.body[0]) {
        Some(Stmt::Expr(e)) => *e,
        other => panic!("not an expression statement: {other:?}"),
    }
}

fn expr(p: &Program, id: ExprId) -> &Expr {
    p.ast.expr(id).unwrap()
}

fn stmt(p: &Program, id: StmtId) -> &Stmt {
    p.ast.stmt(id).unwrap()
}

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

// ---- expressions ----

#[test]
fn binary_operators_follow_precedence_and_associate_left() {
    let p = ok("a + b * c - d;");
    let Expr::Binary { op: BinaryOp::Sub, left, right } = expr(&p, first_expr(&p)) else { panic!() };
    assert!(matches!(expr(&p, *right), Expr::Ident(n) if n == "d"));
    let Expr::Binary { op: BinaryOp::Add, right: mul, .. } = expr(&p, *left) else { panic!() };
    assert!(matches!(expr(&p, *mul), Expr::Binary { op: BinaryOp::Mul, .. }));
}

#[test]
fn logical_operators_bind_looser_than_comparisons() {
    let p = ok("a || b && c == d;");
    let Expr::Logical { right, .. } = expr(&p, first_expr(&p)) else { panic!() };
    let Expr::Logical { right: eq, .. } = expr(&p, *right) else { panic!() };
    assert!(matches!(expr(&p, *eq), Expr::Binary { op: BinaryOp::Eq, .. }));
}

#[test]
fn assignment_associates_right() {
    let p = ok("a = b += c;");
    let Expr::Assign { op: None, value, .. } = expr(&p, first_expr(&p)) else { panic!() };
    assert!(matches!(expr(&p, *value), Expr::Assign { op: Some(BinaryOp::Add), .. }));
}

#[test]
fn new_takes_the_first_argument_list_after_its_callee() {
    let p = ok("new a.b(1).c(2);");
    let Expr::Call { callee, args } = expr(&p, first_expr(&p)) else { panic!() };
    assert_eq!(args.len(), 1);
    let Expr::Member { object, property } = expr(&p, *callee) else { panic!() };
    assert_eq!(property.name, "c");
    let Expr::New { callee, args } = expr(&p, *object) else { panic!() };
    assert_eq!(args.len(), 1);
    assert!(matches!(expr(&p, *callee), Expr::Member { .. }));

    let p = ok("new new X()();");
    let Expr::New { callee, .. } = expr(&p, first_expr(&p)) else { panic!() };
    assert!(matches!(expr(&p, *callee), Expr::New { .. }));

    let p = ok("new X;");
    assert!(matches!(expr(&p, first_expr(&p)), Expr::New { args, .. } if args.is_empty()));
}

#[test]
fn conditional_and_sequence_expressions() {
    let p = ok("a ? b : c ? d : e, f;");
    let Expr::Sequence(list) = expr(&p, first_expr(&p)) else { panic!() };
    assert_eq!(list.len(), 2);
    let Expr::Conditional { alternate, .. } = expr(&p, list[0]) else { panic!() };
    assert!(matches!(expr(&p, *alternate), Expr::Conditional { .. }));
}

#[test]
fn array_literals_keep_holes() {
    let p = ok("[1, , 3, ];");
    let Expr::Array(elements) = expr(&p, first_expr(&p)) else { panic!() };
    assert_eq!(elements.len(), 3);
    assert!(elements[1].is_none());
    let p = ok("[,];");
    assert!(matches!(expr(&p, first_expr(&p)), Expr::Array(e) if e.len() == 1));
}

#[test]
fn object_literals_with_accessors_and_keyword_names() {
    let p = ok("({a: 1, 'b': 2, 3: 3, if: 4, get: 5, set: 6, get x() { return 1; }, set x(v) {}, });");
    let Expr::Paren(inner) = expr(&p, first_expr(&p)) else { panic!() };
    let Expr::Object(props) = expr(&p, *inner) else { panic!() };
    assert_eq!(props.len(), 8);
    assert!(matches!(props[6].value, PropertyValue::Get(_)));
    assert!(matches!(props[7].value, PropertyValue::Set(_)));
    let PropertyValue::Get(getter) = props[6].value else { panic!() };
    assert_eq!(p.ast.function(getter).unwrap().kind, FunctionKind::Getter);
}

#[test]
fn accessors_take_the_right_number_of_parameters() {
    assert!(err("({get a(x) {}});").message.contains("getter"));
    assert!(err("({set a() {}});").message.contains("setter"));
    assert!(err("({set a(x, y) {}});").message.contains("setter"));
}

#[test]
fn property_names_may_repeat() {
    for src in ["({a: 1, a: 2});", "({a: 1, 'a': 2});", "({get a() {}, set a(v) {}});", "({a: 1, get a() {}});", "({get a() {}, get a() {}});"] {
        ok(src);
        ok(&format!("'use strict'; {src}"));
    }
}

#[test]
fn escaped_get_and_set_do_not_start_accessors() {
    assert!(err("({g\\u0065t a() {}});").message.contains("expected ':'"));
    assert!(err("({\\u0073et a(v) {}});").message.contains("expected ':'"));
    ok("({g\\u0065t: 1});");
}

#[test]
fn member_names_may_be_reserved_words() {
    ok("a.if.var.function = b.true;");
    ok("a.\\u0076ar = 1;");
}

// ---- statements ----

#[test]
fn else_if_chains_nest_as_alternates() {
    let p = ok("if (a) x; else if (b) y; else z;");
    let Stmt::If { alternate: Some(alt), .. } = stmt(&p, p.body[0]) else { panic!() };
    let Stmt::If { alternate: Some(last), .. } = stmt(&p, *alt) else { panic!() };
    assert!(matches!(stmt(&p, *last), Stmt::Expr(_)));
}

#[test]
fn for_statements_of_every_form() {
    ok("for (;;) break;");
    ok("for (var i = 0, n = a.length; i < n; i++) {}");
    ok("for (i = 0; i < 10; i++) continue;");
    ok("for (var k in o) {}");
    ok("for (k in o) {}");
    ok("for (a.b in o) {}");
    ok("for (var x = 1 in o) {}");
    ok("for (var i = (a in b); i; ) break;");
    let p = ok("for (var k in o) {}");
    assert!(matches!(stmt(&p, p.body[0]), Stmt::ForIn { target: ForInTarget::Var(_), .. }));
    assert!(err("for (a + b in o) {}").message.contains("for-in"));
    assert!(err("for (var a, b in o) {}").message.contains("expected ';'"));
}

#[test]
fn switch_try_with_labels_and_debugger() {
    ok("switch (x) { case 1: a(); break; default: b(); case 2: }");
    ok("try { a(); } catch (e) { b(); } finally { c(); }");
    ok("try {} finally {}");
    ok("with (o) { a = 1; }");
    ok("outer: for (;;) { inner: for (;;) { continue outer; } }");
    ok("a: b: while (1) { continue a; }");
    ok("a: { break a; }");
    ok("debugger;");
    ok("do x(); while (a) y();");
    assert!(err("switch (x) { default: default: }").message.contains("default"));
    assert!(err("try {}").message.contains("catch or finally"));
}

#[test]
fn break_and_continue_need_a_target() {
    assert!(err("break;").message.contains("break outside"));
    assert!(err("continue;").message.contains("continue outside"));
    assert!(err("switch (x) { case 1: continue; }").message.contains("continue outside"));
    assert!(err("for (;;) { break nowhere; }").message.contains("undefined label"));
    assert!(err("a: { continue a; }").message.contains("does not name a loop"));
    assert!(err("a: while (1) { function f() { break a; } }").message.contains("undefined label"));
    assert!(err("while (1) { function f() { break; } }").message.contains("break outside"));
    assert!(err("a: a: ;").message.contains("duplicate label"));
    assert!(err("a: { a: ; }").message.contains("duplicate label"));
    ok("a: ; a: ;");
}

#[test]
fn return_only_inside_functions() {
    assert!(err("return 1;").message.contains("return outside"));
    ok("function f() { return; }");
    ok("(function () { return 1 })();");
}

#[test]
fn function_declarations_in_blocks_and_branches() {
    ok("{ function f() {} }");
    ok("if (a) function f() {} else function g() {}");
    ok("switch (a) { case 1: function f() {} }");
    ok("l: function f() {}");
    ok("{ l: m: function f() {} }");
    ok("'use strict'; { function f() {} }");
    assert!(strict_err("if (a) function f() {}").message.contains("functions can only be declared"));
    assert!(strict_err("l: function f() {}").message.contains("functions can only be declared"));
    for body in ["while (a) function f() {}", "for (;;) function f() {}", "do function f() {} while (a)", "with (a) function f() {}"] {
        assert!(err(body).message.contains("cannot stand here"), "{body}");
    }
    assert!(err("if (a) l: function f() {}").message.contains("cannot stand here"));
    assert!(err("while (a) l: function f() {}").message.contains("cannot stand here"));
}

// ---- automatic semicolon insertion ----

#[test]
fn semicolons_are_inserted_at_line_breaks_braces_and_the_end() {
    ok("a = 1\nb = 2");
    ok("function f() { return 1 }");
    ok("a = 1");
    let e = err("a = 1 b = 2");
    assert_eq!((e.line, e.column), (1, 7));
}

#[test]
fn restricted_productions_end_at_a_line_break() {
    let p = ok("function f() { return\n1 }");
    let Stmt::FunctionDecl(f) = stmt(&p, p.body[0]) else { panic!() };
    let body = &p.ast.function(*f).unwrap().body;
    assert_eq!(body.len(), 2);
    assert!(matches!(stmt(&p, body[0]), Stmt::Return(None)));

    let p = ok("a\n++b");
    assert_eq!(p.body.len(), 2);
    assert!(matches!(expr(&p, first_expr(&p)), Expr::Ident(_)));

    ok("for (;;) { break\nfoo }");
    assert!(err("throw\nnew Error()").message.contains("line break after throw"));
}

#[test]
fn no_semicolon_is_inserted_inside_a_for_header() {
    assert!(err("for (a\nb\n) {}").message.contains("expected ';'"));
}

// ---- the division and regular expression goals ----

#[test]
fn slash_divides_after_a_value_and_starts_a_regexp_elsewhere() {
    let p = ok("a / b / c;");
    assert!(matches!(expr(&p, first_expr(&p)), Expr::Binary { op: BinaryOp::Div, .. }));
    let p = ok("x = /a\\/b[/]c/gi;");
    let Expr::Assign { value, .. } = expr(&p, first_expr(&p)) else { panic!() };
    let Expr::RegExp { pattern, flags } = expr(&p, *value) else { panic!() };
    assert_eq!(*pattern, utf16("a\\/b[/]c"));
    assert_eq!(flags, "gi");
    ok("if (x) /re/.test(y);");
    ok("{} /re/.test(y);");
    ok("x = y /= 2;");
    ok("x = /=a/;");
    ok("return_ = typeof /x/;");
    ok("a(/x/, /y/);");
    // After a line break a `/` still divides when it can: there is no semicolon to insert.
    let p = ok("a = b\n/c/g");
    assert_eq!(p.body.len(), 1);
}

#[test]
fn tokenize_reads_each_slash_as_the_grammar_decides() {
    let tokens = tokenize("a = b / c; d = /e/g.exec(f);", &Options::default()).unwrap();
    let kinds: Vec<&TokenKind> = tokens.iter().map(|t| &t.kind).collect();
    assert!(kinds.iter().any(|k| matches!(k, TokenKind::RegExp { flags, .. } if flags == "g")));
    assert_eq!(kinds.iter().filter(|k| matches!(k, TokenKind::RegExp { .. })).count(), 1);
    assert!(matches!(kinds.last(), Some(TokenKind::Eof)));
}

// ---- literals ----

fn number(src: &str) -> f64 {
    let p = ok(src);
    let Expr::Number(n) = expr(&p, first_expr(&p)) else { panic!("{src}") };
    *n
}

fn string(src: &str) -> Vec<u16> {
    let p = ok(src);
    let Expr::String(s) = expr(&p, first_expr(&p)) else { panic!("{src}") };
    s.clone()
}

#[test]
fn numeric_literals() {
    assert_eq!(number("0;"), 0.0);
    assert_eq!(number("12.5e-1;"), 1.25);
    assert_eq!(number(".5;"), 0.5);
    assert_eq!(number("5.;"), 5.0);
    assert_eq!(number("1E3;"), 1000.0);
    assert_eq!(number("0x1F;"), 31.0);
    assert_eq!(number("0XfF;"), 255.0);
    assert_eq!(number("0x20000000000001;"), 9007199254740992.0);
    // 2^136 + 2^83 + 1 and 2^150 + 2^97 + 1: a set bit past 128 bits rounds the half-way case up.
    assert_eq!(number(&format!("0x1{}8{}1;", "0".repeat(13), "0".repeat(19))), 2f64.powi(136) + 2f64.powi(84));
    assert_eq!(number(&format!("01{}2{}1;", "0".repeat(17), "0".repeat(31))), 2f64.powi(150) + 2f64.powi(98));
    assert_eq!(number("017;"), 15.0);
    assert_eq!(number("08;"), 8.0);
    assert_eq!(number("09.5;"), 9.5);
    assert_eq!(number("1e400;"), f64::INFINITY);
    assert!(err("3in []").message.contains("immediately after numeric literal"));
    assert!(err("0x;").message.contains("hexadecimal"));
    assert!(err("1.e").message.contains("immediately after"));
    assert!(strict_err("017;").message.contains("octal literals"));
    assert!(strict_err("08;").message.contains("octal literals"));
    assert!(strict_err("({017: 1});").message.contains("octal"));
}

#[test]
fn string_literals_and_escapes() {
    assert_eq!(string(r#""a\tb\n\\\"\'";"#), utf16("a\tb\n\\\"'"));
    assert_eq!(string(r#"'\x41é€';"#), utf16("A\u{e9}\u{20ac}"));
    assert_eq!(string("'a\\\nb';"), utf16("ab"));
    assert_eq!(string("'a\\\r\nb';"), utf16("ab"));
    assert_eq!(string(r"'\0';"), vec![0]);
    assert_eq!(string(r"'\101\7\08';"), vec![0x41, 7, 0, u16::from(b'8')]);
    assert_eq!(string(r"'\q\8';"), utf16("q8"));
    assert_eq!(string("'\u{2028}';"), vec![0x2028]);
    assert_eq!(string("'\u{1F600}';"), utf16("\u{1F600}"));
    assert!(err("'abc").message.contains("unterminated string"));
    assert!(err("'a\nb'").message.contains("unterminated string"));
    assert!(err(r"'\x4';").message.contains("hexadecimal escape"));
    assert!(err(r"'\u12';").message.contains("Unicode escape"));
    assert_eq!(string(r"'\u{41}\u{1F600}\u{0000E9}';"), utf16("A\u{1F600}\u{e9}"));
    for bad in [r"'\u{}';", r"'\u{110000}';", r"'\u{41';", r"'\u{4g}';"] {
        assert!(err(bad).message.contains("Unicode escape"), "{bad}");
    }
    assert!(strict_err(r"'\101';").message.contains("octal escape"));
    assert!(strict_err(r"'\8';").message.contains("octal escape"));
    ok(r"'use strict'; '\0';");
}

#[test]
fn identifiers_with_unicode_and_escapes() {
    let p = ok("var caf\\u00e9 = 1, \u{3b1}\u{3b2} = 2, $_x1 = 3, a\u{300} = 4;");
    let Stmt::Var(decls) = stmt(&p, p.body[0]) else { panic!() };
    let names: Vec<&str> = decls.iter().map(|d| d.name.name.as_str()).collect();
    assert_eq!(names, ["caf\u{e9}", "\u{3b1}\u{3b2}", "$_x1", "a\u{300}"]);
    let p = ok("\\u{61}b;");
    assert!(matches!(expr(&p, first_expr(&p)), Expr::Ident(n) if n == "ab"));
    assert!(err("var \\u0076ar = 1;").message.contains("escaped characters"));
    assert!(err("var a\\u0020b;").message.contains("identifier escape"));
    assert!(err("var \\x41;").message.contains("escape"));
    assert!(err("var a = 1 @ 2;").message.contains("unexpected character '@'"));
    assert!(err("var \u{2e2f};").message.contains("unexpected character"));
    assert!(err("var a\u{2e2f};").message.contains("unexpected character"));
}

#[test]
fn identifier_characters_follow_their_unicode_categories() {
    // Letters (Lu, Ll, Lt, Lm, Lo, Nl) start an identifier; marks (Mn, Mc), decimal digits (Nd)
    // and connector punctuation (Pc) may follow.
    for good in [
        "var \u{995}\u{9cd}\u{9b7} = 1;",
        "var \u{b95}\u{bcd}\u{b95} = 1;",
        "var a\u{9be} = 1;",
        "var x\u{966} = 1;",
        "var a\u{203f}b = 1;",
        "var \u{2b0} = 1;",
        "var \u{2160} = 1;",
        "var \u{1c5} = 1;",
        "var \u{5d0}\u{5b7} = 1;",
    ] {
        ok(good);
    }
    // Other numbers (No) are not identifier characters, and a mark cannot start an identifier.
    for bad in ["var a\u{b2} = 1;", "var x\u{bd} = 1;", "var \u{9be}a = 1;", "var \u{300}a = 1;", "var \u{966} = 1;"] {
        assert!(err(bad).message.contains("unexpected character"), "{bad:?}");
    }
    assert!(parse("var \\u09BEa = 1;", &Options::default()).is_err());
    assert!(parse("var a\\u00B2 = 1;", &Options::default()).is_err());
}

#[test]
fn identifiers_take_the_other_id_start_and_continue_characters() {
    // Other_ID_Start: not letters by category, but ID_Start, so they start and continue a name.
    for c in [0x1885, 0x1886, 0x2118, 0x212E, 0x309B, 0x309C] {
        let ch = char::from_u32(c).unwrap_or_default();
        for src in [format!("var {ch} = 1;"), format!("var a{ch} = 1;"), format!("var \\u{c:04X}\\u{{{c:X}}} = 1;")] {
            let p = ok(&src);
            let Stmt::Var(decls) = stmt(&p, p.body[0]) else { panic!("{src:?}") };
            assert!(decls[0].name.name.contains(ch), "{src:?}");
        }
    }
    // Other_ID_Continue: may follow the first character, but cannot start a name.
    for c in [0xB7, 0x387].into_iter().chain(0x1369..=0x1371).chain([0x19DA, 0x200C, 0x200D, 0x30FB, 0xFF65]) {
        let ch = char::from_u32(c).unwrap_or_default();
        for src in [format!("var a{ch} = 1;"), format!("var a\\u{c:04X} = 1;"), format!("var a\\u{{{c:X}}} = 1;")] {
            ok(&src);
        }
        assert!(err(&format!("var {ch} = 1;")).message.contains("unexpected character"), "{c:X}");
        assert!(err(&format!("var \\u{c:04X} = 1;")).message.contains("identifier escape"), "{c:X}");
        assert!(err(&format!("var \\u{{{c:X}}} = 1;")).message.contains("identifier escape"), "{c:X}");
    }
}

#[test]
fn identifiers_take_the_characters_unicode_17_assigned() {
    // U+088F (a letter, Lo) starts a name; U+1ACF (a combining mark, Mn) may follow the first
    // character but cannot start a name. Unicode 17.0 assigned both.
    for (src, name) in [
        ("var \u{88F} = 1;", "\u{88F}"),
        ("var \\u088F = 1;", "\u{88F}"),
        (r"var \u{88F} = 1;", "\u{88F}"),
        ("var _\u{1ACF} = 1;", "_\u{1ACF}"),
        ("var _\\u1ACF = 1;", "_\u{1ACF}"),
        (r"var _\u{1ACF} = 1;", "_\u{1ACF}"),
    ] {
        let p = ok(src);
        let Stmt::Var(decls) = stmt(&p, p.body[0]) else { panic!("{src:?}") };
        assert_eq!(decls[0].name.name, name, "{src:?}");
    }
    assert!(err("var \u{1ACF} = 1;").message.contains("unexpected character"));
    assert!(err("var \\u1ACF = 1;").message.contains("identifier escape"));
    assert!(err(r"var \u{1ACF} = 1;").message.contains("identifier escape"));
}

#[test]
fn identifiers_refuse_pattern_syntax_surrogates_and_bad_escapes() {
    // U+2E2F is a letter by category but Pattern_Syntax; U+2010, U+00B6 and U+2190 are
    // punctuation and symbols in Pattern_Syntax.
    for c in [0x2E2F, 0x2010, 0xB6, 0x2190, 0x3001] {
        let ch = char::from_u32(c).unwrap_or_default();
        assert!(err(&format!("var {ch} = 1;")).message.contains("unexpected character"), "{c:X}");
        assert!(err(&format!("var a{ch} = 1;")).message.contains("unexpected character"), "{c:X}");
        assert!(err(&format!("var a\\u{c:04X} = 1;")).message.contains("identifier escape"), "{c:X}");
        assert!(err(&format!("var \\u{{{c:X}}} = 1;")).message.contains("identifier escape"), "{c:X}");
    }
    for bad in [r"var \uD800 = 1;", r"var a\uDC00 = 1;", r"var a\u{D800} = 1;", r"var a\u{110000} = 1;", r"var a\u{} = 1;"] {
        assert!(parse(bad, &Options::default()).is_err(), "{bad}");
    }
    // A supplementary letter is one identifier character, at the start and after a number.
    let p = ok("var \u{10000}\u{1D7CE} = 1;");
    let Stmt::Var(decls) = stmt(&p, p.body[0]) else { panic!() };
    assert_eq!(decls[0].name.name, "\u{10000}\u{1D7CE}");
    assert!(err("1\u{10000};").message.contains("immediately after numeric literal"));
}

/// The code point ranges of `Pattern_Syntax` and `Pattern_White_Space` as Unicode's PropList lists
/// them; Unicode does not change either set.
fn pattern_syntax_and_white_space() -> Vec<std::ops::RangeInclusive<u32>> {
    let pattern = "0009..000D 0020 0021..002F 003A..0040 005B..005E 0060 007B..007E 0085 00A1..00A7 00A9 00AB..00AC 00AE \
                   00B0..00B1 00B6 00BB 00BF 00D7 00F7 200E..200F 2010..2027 2028..2029 2030..203E 2041..2053 2055..205E \
                   2190..245F 2500..2775 2794..2BFF 2E00..2E7F 3001..3003 3008..3020 3030 FD3E..FD3F FE45..FE46";
    pattern
        .split_whitespace()
        .map(|range| {
            let (lo, hi) = range.split_once("..").unwrap_or((range, range));
            let (Ok(lo), Ok(hi)) = (u32::from_str_radix(lo, 16), u32::from_str_radix(hi, 16)) else { panic!("{range}") };
            lo..=hi
        })
        .collect()
}

#[test]
fn no_pattern_syntax_or_white_space_character_is_an_identifier_character() {
    use crate::chars::{is_id_part, is_id_start};
    // `$` is Pattern_Syntax but an identifier character in scripts.
    let mut checked = 0;
    for range in pattern_syntax_and_white_space() {
        for c in range.filter(|&c| c != 0x24) {
            assert!(!is_id_start(c) && !is_id_part(c), "U+{c:04X}");
            checked += 1;
        }
    }
    assert_eq!(checked, 2760 + 11 - 1);
    for c in 0xD800..=0xDFFF {
        assert!(!is_id_start(c) && !is_id_part(c), "U+{c:04X}");
    }
    assert!(!is_id_start(0x110000) && !is_id_part(0x110000));
}

#[test]
fn identifier_characters_are_unicode_17_id_start_and_id_continue() {
    use crate::chars::{is_id_part, is_id_start};
    use unicode_properties::{GeneralCategory as G, UnicodeGeneralCategory};
    // Unicode Standard Annex #31: ID_Start is the categories Lu, Ll, Lt, Lm, Lo and Nl plus
    // Other_ID_Start; ID_Continue adds Mn, Mc, Nd, Pc and Other_ID_Continue; both leave out
    // Pattern_Syntax and Pattern_White_Space. Other_ID_Start and Other_ID_Continue as PropList 17.0
    // lists them.
    let other_start = [0x1885..=0x1886, 0x2118..=0x2118, 0x212E..=0x212E, 0x309B..=0x309C];
    let other_continue = [0xB7..=0xB7, 0x387..=0x387, 0x1369..=0x1371, 0x19DA..=0x19DA, 0x200C..=0x200D, 0x30FB..=0x30FB, 0xFF65..=0xFF65];
    let mut excluded = vec![false; 0x11_0000];
    for range in pattern_syntax_and_white_space() {
        for c in range {
            excluded[c as usize] = true;
        }
    }
    let (mut id_starts, mut id_continues) = (0, 0);
    for c in (0..=0x10_FFFF_u32).filter(|c| !(0xD800..=0xDFFF).contains(c)) {
        let Some(ch) = char::from_u32(c) else { panic!("U+{c:04X}") };
        let g = ch.general_category();
        let letter = matches!(g, G::UppercaseLetter | G::LowercaseLetter | G::TitlecaseLetter | G::ModifierLetter | G::OtherLetter | G::LetterNumber);
        let id_start = (letter || other_start.iter().any(|r| r.contains(&c))) && !excluded[c as usize];
        let continuing = matches!(g, G::NonspacingMark | G::SpacingMark | G::DecimalNumber | G::ConnectorPunctuation);
        let id_continue = (id_start || continuing || other_continue.iter().any(|r| r.contains(&c))) && !excluded[c as usize];
        id_starts += usize::from(id_start);
        id_continues += usize::from(id_continue);
        // Scripts add `$` and `_` at the start and `$` later (ECMA-262, Names and Keywords).
        assert_eq!(is_id_start(c), id_start || c == 0x24 || c == 0x5F, "start U+{c:04X}");
        assert_eq!(is_id_part(c), id_continue || c == 0x24, "part U+{c:04X}");
    }
    // The totals of ID_Start and ID_Continue in DerivedCoreProperties 17.0.
    assert_eq!((id_starts, id_continues), (145_916, 149_240));
}

#[test]
fn comments_and_line_terminators() {
    let p = ok("a /* one\ntwo */ b");
    assert_eq!(p.body.len(), 2, "a comment with a line break separates statements");
    assert!(err("a /* one two */ b").message.contains("unexpected identifier 'b'"));
    assert!(err("/* open").message.contains("unterminated comment"));
    ok("// only a comment");
    ok("a\u{2028}b\u{2029}c\r\nd\re");
    ok("\u{feff}var\u{a0}a\u{3000}=\u{b}1\u{c};");
}

// ---- regular expression literals ----

#[test]
fn regular_expression_flags_and_patterns_are_checked() {
    for good in [
        "/a|b*c+?d{2}e{1,}f{1,3}?/",
        "/[a-z\\d-]/",
        "/[\\d-z]/",
        "/]/",
        "/{/",
        "/a{,}/",
        "/x{a}/",
        "/(?:a)(?=b)(?!c)/",
        "/(?=a)*/",
        "/\\1(a)/",
        "/[]/",
        "/[^]/",
        "/\\cJ\\c/",
        "/[\\b-\\n]/",
        "/a/gim",
    ] {
        ok(&format!("x = {good};"));
    }
    for (bad, message) in [
        ("/(/", "unterminated group"),
        ("/a)/", "unmatched"),
        ("/?/", "nothing to repeat"),
        ("/a**/", "nothing to repeat"),
        ("/^*/", "nothing to repeat"),
        ("/{1}/", "nothing to repeat"),
        ("/a{2,1}/", "out of order"),
        ("/[z-a]/", "out of order"),
        ("/(?<a>x)/", "invalid group"),
        ("/[a/", "unterminated regular expression"),
        ("/a\\/", "unterminated regular expression"),
        ("/a\n/", "unterminated regular expression"),
        ("/a/gg", "flags"),
        ("/a/x", "flags"),
        ("/a/\\u0067", "flags"),
    ] {
        let e = err(&format!("x = {bad};"));
        assert!(e.message.contains(message), "{bad}: {}", e.message);
        assert_eq!((e.line, e.column), (1, 5), "{bad}");
    }
}

// ---- strict mode ----

#[test]
fn use_strict_makes_the_rest_strict_mode_code() {
    assert!(ok("'use strict'; a;").strict);
    assert!(ok("\"use strict\"\na;").strict);
    assert!(!ok("'use\\x20strict'; a;").strict, "a directive with an escape is not \"use strict\"");
    assert!(!ok("('use strict'); a;").strict);
    assert!(!ok("a; 'use strict';").strict);
    assert!(ok("'other'; 'use strict'; a;").strict);
    assert!(!ok("'use strict' + 1;").strict);
    let p = ok("function f() { 'use strict'; } function g() {}");
    let funcs = &p.ast.functions;
    assert!(funcs[0].strict && !funcs[1].strict && !p.strict);
    assert!(parse("with (a) {}", &Options { strict: true, ..Options::default() }).is_err());
}

#[test]
fn strict_mode_early_errors() {
    assert!(strict_err("with (a) {}").message.contains("with statements"));
    assert!(strict_err("delete a;").message.contains("delete"));
    assert!(strict_err("delete ((a));").message.contains("delete"));
    ok("'use strict'; delete a.b; delete a[0];");
    for target in ["eval = 1;", "arguments++;", "--eval;", "arguments += 1;", "(eval) = 1;", "for (eval in o) {}"] {
        assert!(strict_err(target).message.contains("cannot assign"), "{target}");
    }
    for binding in ["var eval;", "function arguments() {}", "function f(eval) {}", "try {} catch (arguments) {}", "(function eval() {});"] {
        assert!(strict_err(binding).message.contains("cannot be declared"), "{binding}");
    }
    for word in ["implements", "interface", "let", "package", "private", "protected", "public", "static", "yield"] {
        ok(&format!("var {word} = 1;"));
        assert!(strict_err(&format!("var {word} = 1;")).message.contains("reserved word"), "{word}");
        assert!(strict_err(&format!("{word};")).message.contains("reserved word"), "{word}");
    }
    assert!(strict_err("function f(a, a) {}").message.contains("duplicate parameter"));
    ok("function f(a, a) {}");
    let e = strict_err("for (var x = 1 in o) {}");
    assert!(e.message.contains("initializer"), "{}", e.message);
    assert_eq!((e.line, e.column), (2, 16));
    ok("'use strict'; for (var x in o) {}");
}

#[test]
fn a_function_made_strict_by_its_own_prologue_checks_its_name_and_parameters() {
    let e = err("function f(a, a) { 'use strict'; }");
    assert!(e.message.contains("duplicate parameter"));
    assert_eq!((e.line, e.column), (1, 15));
    assert!(err("function eval() { 'use strict'; }").message.contains("cannot be declared"));
    assert!(err("function f(arguments) { 'use strict'; }").message.contains("cannot be declared"));
    assert!(err("function static() { 'use strict'; }").message.contains("reserved word"));
    assert!(err("function f() { '\\01'; 'use strict'; }").message.contains("octal escape"));
    assert!(err("'\\01'; 'use strict';").message.contains("octal escape"));
    assert!(err("function f() { 'use strict'; 010 }").message.contains("octal literals"));
    assert!(err("function f() { 'use strict'; var public; }").message.contains("reserved word"));
}

// ---- assignment targets ----

#[test]
fn invalid_assignment_targets() {
    for bad in ["1 = 2;", "a + b = c;", "(a, b) = 1;", "a = 1 = 2;", "++1;", "1++;", "--a.b++;", "this = 1;", "'a' += 1;", "(a = 1) = 2;"] {
        let e = err(bad);
        assert!(e.message.contains("invalid"), "{bad}: {}", e.message);
    }
    ok("(a) = 1; a.b = 1; a[0] += 2; a.b++; --a[1]; (a.b) = c;");
    // Assigning to a call's result fails at run time outside strict mode code, and is a
    // SyntaxError in it.
    ok("f() = 1; f()++; for (f() in o) {}");
    for call in ["f() = 1;", "f() += 1;", "++f();", "f()--;", "(f()) = 1;", "for (f() in o) {}"] {
        assert!(strict_err(call).message.contains("invalid") || strict_err(call).message.contains("for-in"), "{call}");
    }
}

// ---- preprocessor lines ----

#[test]
fn preprocessor_lines_are_read_as_directives() {
    let src = "#target vectorcraft\n  #include \"lib/a.jsxinc\"\n//@includepath \"~/x;~/y\"\n#strict on\n#unknown x\nvar a = 1;\n#targetengine main";
    let p = parse(src, &with_preprocessor()).unwrap();
    let names: Vec<(&str, u32, &str)> = p.directives.iter().map(|d| (d.name.as_str(), d.line, d.value.as_str())).collect();
    assert_eq!(
        names,
        [
            ("target", 1, "vectorcraft"),
            ("include", 2, "\"lib/a.jsxinc\""),
            ("includepath", 3, "\"~/x;~/y\""),
            ("strict", 4, "on"),
            ("unknown", 5, "x"),
            ("targetengine", 7, "main"),
        ]
    );
    assert_eq!(p.directives[2].form, "//@");
    assert!(p.directives[4].known.is_none());
    assert_eq!(p.body.len(), 1);
    assert!(parse(src, &Options::default()).is_err(), "without preprocessor lines a # is an error");
}

#[test]
fn a_hash_away_from_the_start_of_a_line_is_an_error() {
    let e = parse("var a = 1; #include x", &with_preprocessor()).unwrap_err();
    assert!(e.message.contains("unexpected character '#'"));
    assert_eq!((e.line, e.column), (1, 12));
    assert!(parse("/* c */ #include x", &with_preprocessor()).is_err());
    // A comment naming no directive stays a comment.
    assert!(parse("//@other\n", &with_preprocessor()).unwrap().directives.is_empty());
}

#[test]
fn a_directive_between_tokens_keeps_its_place_in_the_token_list() {
    let tokens = tokenize("a\n#include x\nb", &with_preprocessor()).unwrap();
    let kinds: Vec<String> = tokens
        .iter()
        .map(|t| match &t.kind {
            TokenKind::Ident(n) => n.clone(),
            TokenKind::Directive(d) => format!("#{}", d.name),
            TokenKind::Eof => "eof".into(),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(kinds, ["a", "#include", "b", "eof"]);
    assert!(tokens[2].newline_before);
}

// ---- positions ----

#[test]
fn errors_carry_line_and_column() {
    let e = err("var a = 1;\nvar b = ];");
    assert_eq!((e.line, e.column, e.offset), (2, 9, 19));
    assert_eq!(e.to_string(), "SyntaxError: unexpected token ']' (line 2, column 9)");
    let e = err("a\r\nb\r\n  'x");
    assert_eq!((e.line, e.column), (3, 3));
    let e = err("a\u{2028}  )");
    assert_eq!((e.line, e.column), (2, 3));
    let e = err("'\u{1F600}' )");
    assert_eq!((e.line, e.column), (1, 6), "columns count UTF-16 code units");
    let e = err("x = 'a\\\nb' )");
    assert_eq!((e.line, e.column), (2, 4), "a line continuation in a string counts its line");
}

#[test]
fn line_count_and_spans() {
    let p = ok("a;\r\nb;\nc;");
    assert_eq!(p.line_count(), 3);
    assert_eq!(ok("a;\n").line_count(), 1);
    assert_eq!(ok("").line_count(), 0);
    let span = p.ast.stmt_span(p.body[2]);
    assert_eq!(p.line_column(span.start), (3, 1));
    assert_eq!((span.start, span.end), (7, 9));
}

// ---- nesting and limits ----

#[test]
fn deep_nesting_gives_a_syntax_error_not_a_stack_overflow() {
    const N: usize = 100_000;
    let cases = [
        format!("{}1{}", "(".repeat(N), ")".repeat(N)),
        format!("{}1{}", "[".repeat(N), "]".repeat(N)),
        format!("x = {}{}", "{a:".repeat(N), "}".repeat(N)),
        format!("{}{}", "{".repeat(N), "}".repeat(N)),
        format!("{}{}", "function f(){".repeat(N), "}".repeat(N)),
        format!("{}1", "a ? b : ".repeat(N)),
        format!("{}x{}", "f(".repeat(N), ")".repeat(N)),
        format!("{}x", "if (a) ".repeat(N)),
    ];
    for src in &cases {
        let e = err(src);
        assert_eq!(e.message, TOO_DEEP, "{}", &src[..20]);
        assert_eq!(e.line, 1);
        assert!(e.column > 1);
    }
    let e = err(&format!("x = /{}a{}/;", "(".repeat(N), ")".repeat(N)));
    assert!(e.message.contains(TOO_DEEP));
}

#[test]
fn nesting_up_to_the_limit_parses() {
    let depth = 50;
    ok(&format!("{}1{};", "(".repeat(depth), ")".repeat(depth)));
    ok(&format!("{}{}", "{".repeat(depth), "}".repeat(depth)));
    ok(&format!("x = {}{};", "[".repeat(depth), "]".repeat(depth)));
}

#[test]
fn long_flat_runs_do_not_nest() {
    const N: usize = 100_000;
    ok(&format!("x = 1{};", " + 1".repeat(N)));
    ok(&format!("x = {}y;", "!".repeat(N)));
    ok(&format!("{}x = 1;", "a = ".repeat(N)));
    ok(&format!("if (a) b;{} else c;", " else if (a) b;".repeat(N)));
    ok(&format!("{}x;", (0..N).map(|i| format!("l{i}: ")).collect::<String>()));
    ok(&format!("x{};", ".y".repeat(N)));
    ok(&format!("x{};", "()".repeat(N)));
    ok(&"a;\n".repeat(N));
}

#[test]
fn long_label_chains_and_parameter_lists_are_checked_in_linear_time() {
    // Each check looks a name up in a hash set or map. Comparing every name with every earlier
    // one would take about 2 * 10^10 steps at this size.
    const N: usize = 200_000;
    let labels: String = (0..N).map(|i| format!("l{i}: ")).collect();
    let body: String = (0..N).map(|i| format!("m{i}: break l{i}; ")).collect();
    ok(&format!("{labels}for (;;) {{ {body}continue l0; }}"));
    assert!(err(&format!("{labels}l0: x;")).message.contains("duplicate label"));
    assert!(err(&format!("{labels}for (;;) {{ m: break l{N}; }}")).message.contains("undefined label"));
    let params = (0..N).map(|i| format!("p{i}")).collect::<Vec<_>>().join(", ");
    ok(&format!("function f({params}) {{ 'use strict'; }}"));
    assert!(err(&format!("function f({params}, p0) {{ 'use strict'; }}")).message.contains("duplicate parameter"));
}

#[test]
fn nesting_to_the_default_limit_fits_a_small_stack() {
    // The deepest nesting each production allows, parsed on a thread with a 1 MiB stack (the
    // stack a wasm build or a secondary thread may have).
    let n = crate::DEFAULT_MAX_DEPTH;
    let cases = [
        format!("x = {}1{};", "(".repeat(n), ")".repeat(n)),
        format!("x = {}1{};", "[".repeat(n), "]".repeat(n)),
        format!("x = {}1{};", "{a:".repeat(n), "}".repeat(n)),
        format!("{}{}", "function f(){".repeat(n), "}".repeat(n)),
        format!("{}x;", "if (a) ".repeat(n)),
        format!("x = {}1;", "a ? b : ".repeat(n)),
        format!("{}x{};", "f(".repeat(n), ")".repeat(n)),
        format!("{}x{};", "a[".repeat(n), "]".repeat(n)),
    ];
    let handle = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || cases.iter().map(|src| parse(src, &Options::default()).map(|_| ()).map_err(|e| e.message)).collect::<Vec<_>>())
        .unwrap();
    for result in handle.join().unwrap() {
        // Each either parses or stops at the limit; neither overflows the stack.
        assert!(result.is_ok() || result == Err(TOO_DEEP.to_string()), "{result:?}");
    }
}

#[test]
fn sources_over_the_length_limit_are_refused() {
    let options = Options { max_source_units: 10, ..Options::default() };
    assert_eq!(parse("a = 1;", &options).map(|_| ()), Ok(()));
    assert_eq!(parse("a = 1 + 2 + 3;", &options).unwrap_err().message, crate::TOO_LONG);
    assert_eq!(parse("'\u{e9}\u{e9}\u{e9}\u{e9}';", &options).map(|_| ()), Ok(()), "the limit counts UTF-16 code units");
}

// ---- robustness ----

/// Fragments of the language, put together at random so the parser reaches its deeper states.
pub(crate) const FRAGMENTS: &[&str] = &[
    "var ",
    "function ",
    "f",
    "(",
    ")",
    "{",
    "}",
    "[",
    "]",
    ";",
    ",",
    "=",
    "+",
    "-",
    "/",
    "/=",
    "*",
    "?",
    ":",
    "!",
    "++",
    "--",
    "if",
    "else",
    "for",
    "in",
    "while",
    "do",
    "return",
    "break",
    "continue",
    "new",
    "delete",
    "typeof",
    "this",
    "with",
    "switch",
    "case",
    "default",
    "try",
    "catch",
    "finally",
    "throw",
    "get",
    "set",
    "'s'",
    "\"\\x4",
    "1",
    "0x",
    "017",
    ".5",
    "1e",
    "/re/g",
    "a:",
    "\n",
    " ",
    "'use strict';",
    "#include x\n",
    "//c\n",
    "/*",
    "*/",
    "\\u0061",
    "\u{2028}",
    "eval",
    "arguments",
    ".",
    "x",
];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn arbitrary_text_never_panics(src in ".{0,200}") {
        let _ = parse(&src, &Options::default());
        let _ = parse(&src, &Options { strict: true, preprocessor_lines: true, ..Options::default() });
    }

    #[test]
    fn arbitrary_fragments_never_panic(parts in proptest::collection::vec(0..FRAGMENTS.len(), 0..80)) {
        let src: String = parts.iter().map(|&i| FRAGMENTS[i]).collect();
        for options in [Options::default(), Options { strict: true, preprocessor_lines: true, max_depth: 8, ..Options::default() }] {
            match parse(&src, &options) {
                Ok(p) => {
                    for (_, span) in &p.ast.exprs {
                        prop_assert!(span.start <= span.end && span.end as usize <= src.encode_utf16().count());
                    }
                }
                Err(e) => prop_assert!(e.line >= 1 && e.column >= 1),
            }
        }
    }

    #[test]
    fn arbitrary_utf16_never_panics(units in proptest::collection::vec(any::<u16>(), 0..120)) {
        let _ = crate::parse_utf16(&units, &Options::default());
    }
}
