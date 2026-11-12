//! The JavaScript parser's lexer (`vectorcraft-js`) and the script checker's (`vectorcraft-scriptcheck`)
//! read the same scripts the same way: both crates stand alone, so each has its own lexer, and these
//! tests run both over one set of scripts and fail on any disagreement about the lines a script has,
//! its preprocessor lines, where a string, comment or regular expression that is never closed
//! starts, and the characters a name may hold. The scripts stay inside ECMAScript 3, the language both read (no XML literals, which
//! only the checker reads).

use vectorcraft_js::{Options, TokenKind, parse, tokenize};
use vectorcraft_scriptcheck::{Limits, check};

/// The problems the checker reports from its lexer, which the parser reports as a `SyntaxError`.
const LEXICAL: [(&str, &str); 3] = [
    ("unterminatedString", "unterminated string"),
    ("unterminatedComment", "unterminated comment"),
    ("unterminatedRegExp", "unterminated regular expression"),
];

/// Scripts both lexers read without a problem, each with a `/` or a quote in a place where reading
/// it the other way would change the tokens.
const VALID: &[&str] = &[
    "var a = b / c / d;\nvar r = /ab+c/gi;\n",
    "if (x) /re/.test(y);\nwhile (z) /q/g.exec(w);\n",
    "function f() { return /x/; }\nvar g = function () {} / 2;\n",
    "{}\n/re/.test(s);\n",
    "x = {a: 1} / 2;\n",
    "a = b\n/c/g\n",
    "s = 'it\\'s \"q\" // not a comment';\nt = \"/* nor this */\";\n",
    "s = 'line\\\ncontinued';\nu = 1;\n",
    "x = a++ / 2;\ny = (b) / 2;\nz = c[0] / 2;\n",
    "r = /[/]/;\nq = /\\//;\n",
    "#target vectorcraft\n#include \"lib.jsxinc\"\n//@includepath \"~/x\"\nvar a = 1;\n#targetengine session\n",
    "a = 1;\r\nb = 2;\r\nc = 3;",
    "a = 1;\u{2028}b = 2;\u{2029}c = 3;\n",
    "typeof /x/ === 'object';\nvoid /y/;\n",
    "switch (x) { case /a/.test(y): break; }\n",
    "x = y ? /a/ : /b/;\n",
    "label: for (;;) { break label; }\n/* multi\nline */ z = 1 / 2;\n",
    "var o = { re: /x/, d: 4 / 2 };\n",
    "x = [/a/, 1 / 2];\n",
    "if (a) {\n} else /x/.test(b);\n",
    "a = 'q' + \"r\";\n// a comment with a 'quote\n",
    "x = a\n++b\n",
    "for (var i = 0; i < n / 2; i++) { s += /\\d/.exec(t[i]) ? 1 : 0; }\n",
    "",
    "\n\n",
];

/// Scripts with a string, comment or regular expression that is never closed: the problem the
/// checker gives and the line it starts on.
const INVALID: &[(&str, &str, usize)] = &[
    ("var a = 1;\nvar s = 'abc;\n", "unterminatedString", 2),
    ("a = 1;\n/* never closed\nb = 2;\n", "unterminatedComment", 2),
    ("x = 1;\ny = /abc\n", "unterminatedRegExp", 2),
    ("if (x)\n  /abc\n", "unterminatedRegExp", 2),
    ("a = 1;\r\nb = \"x\r\n", "unterminatedString", 2),
    ("q = 2 / 3;\nr = /[/]\n", "unterminatedRegExp", 2),
];

fn js_options() -> Options {
    Options { preprocessor_lines: true, ..Options::default() }
}

#[test]
fn both_lexers_count_the_same_lines_and_read_the_same_directives() {
    for src in VALID {
        let program = parse(src, &js_options()).unwrap_or_else(|e| panic!("{src:?}: {e}"));
        let report = check("script.jsx", src.as_bytes(), None, &Limits::default());
        assert_eq!(report.lines, program.line_count() as usize, "lines of {src:?}");
        let lexical: Vec<_> = report.problems.iter().filter(|p| LEXICAL.iter().any(|(code, _)| *code == p.code)).collect();
        assert!(lexical.is_empty(), "{src:?}: the checker reports {lexical:?}");
        let theirs: Vec<(String, usize)> = report.directives.iter().map(|d| (d.name.to_string(), d.line)).collect();
        let ours: Vec<(String, usize)> = program.directives.iter().map(|d| (d.name.to_lowercase(), d.line as usize)).collect();
        assert_eq!(ours, theirs, "directives of {src:?}");
    }
}

#[test]
fn both_lexers_find_the_same_unclosed_token_on_the_same_line() {
    for (src, code, line) in INVALID {
        let report = check("script.jsx", src.as_bytes(), None, &Limits::default());
        let theirs = report.problems.iter().find(|p| LEXICAL.iter().any(|(c, _)| *c == p.code));
        let theirs = theirs.unwrap_or_else(|| panic!("{src:?}: the checker reports no lexical problem"));
        assert_eq!((theirs.code, theirs.line), (*code, Some(*line)), "{src:?}");
        let ours = parse(src, &js_options()).expect_err(src);
        let message = LEXICAL.iter().find(|(c, _)| c == code).map(|(_, m)| *m).unwrap();
        assert!(ours.message.contains(message), "{src:?}: {}", ours.message);
        assert_eq!(ours.line as usize, *line, "{src:?}");
    }
}

/// Characters on both sides of the identifier rule: `Other_ID_Start`, `Other_ID_Continue`, connector
/// punctuation, marks, digits and letters of other scripts, a letter and a mark Unicode 17.0
/// assigned, and a superscript digit, a `Pattern_Syntax` letter and punctuation that no name holds.
const NAME_CHARS: &[char] = &[
    '\u{AA}', '\u{B2}', '\u{B7}', '\u{D7}', '\u{300}', '\u{387}', '\u{660}', '\u{88F}', '\u{1369}', '\u{1885}', '\u{19DA}', '\u{1ACF}', '\u{200C}',
    '\u{200D}', '\u{203F}', '\u{2040}', '\u{2118}', '\u{212E}', '\u{2160}', '\u{2E2F}', '\u{3005}', '\u{309B}', '\u{30FB}', '\u{FE33}', '\u{FF10}',
    '\u{FF65}',
];

/// Whether the parser reads `src` as one identifier spelling `name`.
fn one_identifier(src: &str, name: &str) -> bool {
    let want = TokenKind::Ident(name.to_string());
    tokenize(src, &js_options()).is_ok_and(|tokens| tokens.iter().filter(|t| t.kind != TokenKind::Eof).map(|t| &t.kind).eq([&want]))
}

/// Whether the checker lists the member `app.<src>` as `app.<name>`.
fn checker_member(src: &str, name: &str) -> bool {
    let report = check("script.jsx", format!("app.{src};\n").as_bytes(), None, &Limits::default());
    report.members.iter().any(|m| m.name == format!("app.{name}"))
}

#[test]
fn both_lexers_take_the_same_characters_into_names() {
    assert!(checker_member("a\u{2118}b", "a\u{2118}b") && one_identifier("a\u{2118}b", "a\u{2118}b"));
    assert!(checker_member("\u{88F}\u{1ACF}", "\u{88F}\u{1ACF}") && one_identifier("\u{88F}\u{1ACF}", "\u{88F}\u{1ACF}"));
    for &c in NAME_CHARS {
        let escape = format!("\\u{:04X}", u32::from(c));
        for (src, name) in [
            (format!("a{c}b"), format!("a{c}b")),
            (format!("{c}b"), format!("{c}b")),
            (format!("a{escape}b"), format!("a{c}b")),
            (format!("{escape}b"), format!("{c}b")),
        ] {
            assert_eq!(checker_member(&src, &name), one_identifier(&src, &name), "{src:?} (U+{:04X})", u32::from(c));
        }
    }
}

#[test]
fn the_parser_reads_a_regexp_wherever_the_checker_does() {
    // Each `/` that divides has no second `/` after it on its line, so a lexer that read one as a
    // regular expression would report it as never closed. The checker reports nothing, and the
    // parser reads regular expressions on lines 2 and 3 only.
    let src = "a = b / c;\nr = /x/;\nif (a) /y/.test(b);\nz = (1) / 2;\n";
    let tokens = tokenize(src, &js_options()).unwrap();
    let lines: Vec<u32> = tokens.iter().filter(|t| matches!(t.kind, TokenKind::RegExp { .. })).map(|t| t.line).collect();
    assert_eq!(lines, [2, 3]);
    let report = check("script.jsx", src.as_bytes(), None, &Limits::default());
    assert!(report.problems.iter().all(|p| !LEXICAL.iter().any(|(c, _)| *c == p.code)), "{:?}", report.problems);
}
