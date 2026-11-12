use proptest::prelude::*;

use crate::{JsValue, LimitKind, Limits, Options, Realm, RunErrorKind, parse};

fn e4x() -> Options {
    Options { e4x: true, ..Options::default() }
}

fn realm() -> Realm {
    Realm::with_options(e4x(), Limits::default())
}

fn limited(limits: Limits) -> Realm {
    Realm::with_options(e4x(), limits)
}

fn run(src: &str) -> JsValue {
    match realm().run(src) {
        Ok(v) => v,
        Err(e) => panic!("{src}\n{e:?}"),
    }
}

fn string(src: &str) -> String {
    match run(src) {
        JsValue::String(s) => s,
        other => panic!("{src}: {other:?}"),
    }
}

fn truth(src: &str) {
    assert_eq!(run(src), JsValue::Bool(true), "{src}");
}

fn thrown_name(src: &str) -> String {
    string(&format!("try {{ {src}; 'no error' }} catch (e) {{ e.name }}"))
}

const BOOKS: &str = "var xml = <bookstore>\
    <book category=\"COOKING\"><title lang=\"en\">Cookbook</title><author>Farmer</author><year>1896</year></book>\
    <book category=\"CHILDREN\"><title lang=\"en\">Oz</title><author>Baum</author><year>1900</year></book>\
    <book category=\"CHILDREN\"><title lang=\"en\">Alice</title><author>A</author><author>B</author><author>C</author><year>1865</year></book>\
    <book category=\"MUSIC\"><title lang=\"en\">Opera</title><author>Walbrook</author><year>1922</year></book>\
    </bookstore>;";

const TOP: &str = "var x = new XML('<top>\\n<one>one text</one>\\n<two>\\ntwo text\\n<inside>inside text</inside>\\n</two>\\ntop text\\n</top>');";

// ---- default isolation ----

#[test]
fn without_the_option_e4x_syntax_is_rejected_and_the_globals_are_absent() {
    for src in ["<x/>", "x..y", "x.@a", "x.(y)", "x.*", "a::b", "default xml namespace = 'u';", "for each (var v in o);"] {
        assert!(parse(src, &Options::default()).is_err(), "{src}");
        assert!(parse(src, &e4x()).is_ok(), "{src}");
    }
    let mut plain = Realm::new();
    for name in ["XML", "XMLList", "Namespace", "QName", "isXMLName", "setDefaultXMLNamespace"] {
        assert_eq!(plain.run(&format!("typeof {name}")), Ok(JsValue::String("undefined".into())), "{name}");
    }
}

#[test]
fn ordinary_expressions_mean_the_same_with_the_option() {
    truth("1 < 2 && 3 > 2 && 4 / 2 / 1 === 2 && /a<b/.test('a<b')");
    truth("var a = 1, b = 2; a<b && !(b<a)");
    truth("var o = {x: 1}; var t = o.x ? 1 : 2; t === 1");
    truth("var f = new Function('return (<a>1</a>).toString()'); f() === '1'");
    truth("eval('(<a><b/></a>).b.length()') === 1");
    let mut plain = Realm::new();
    assert_eq!(plain.run("try { eval('<x/>') } catch (e) { e.name }"), Ok(JsValue::String("SyntaxError".into())));
    assert_eq!(plain.run("try { new Function('return <x/>') } catch (e) { e.name }"), Ok(JsValue::String("SyntaxError".into())));
}

// ---- results stated by ECMA-357 ----

#[test]
fn literals_match_the_standard_examples() {
    assert_eq!(string("var person = <person><name>John</name><age>25</age></person>; person.name.toString()"), "John");
    assert_eq!(run("<><name>Phil</name><age>35</age><hobby>skiing</hobby></>.length()"), JsValue::Number(3.0));
    let item = "var item = <item><description>Laptop Computer</description><price>2799.95</price><quantity>1</quantity></item>;";
    assert_eq!(
        string(&format!("{item} 'Description stored as ' + item.description.toXMLString()")),
        "Description stored as <description>Laptop Computer</description>"
    );
    assert_eq!(string(&format!("{item} 'Thank you for purchasing a ' + item.description + '!'")), "Thank you for purchasing a Laptop Computer!");
}

#[test]
fn parsed_markup_has_the_expected_nodes() {
    let src = "XML.ignoreComments = false; var t = new XML('<a p=\"1\" q=\"&lt;\">x<!--c--><?pi d?><b/>&amp;y<![CDATA[<z>]]></a>');";
    truth(&format!("{src} t.children().length() === 5"));
    truth(&format!("{src} t.children()[0].nodeKind() === 'text' && t.children()[0].toString() === 'x'"));
    truth(&format!("{src} t.children()[1].nodeKind() === 'comment' && t.comments().length() === 1"));
    truth(&format!("{src} t.children()[2].nodeKind() === 'processing-instruction' && t.children()[2].name().localName === 'pi'"));
    truth(&format!("{src} t.children()[3].nodeKind() === 'element'"));
    truth(&format!("{src} t.children()[4].toString() === '&y<z>'"));
    truth(&format!("{src} t.@p == '1' && t.@q == '<' && t.attributes().length() === 2 && t.@p.nodeKind() === 'attribute'"));
}

#[test]
fn serialization_matches_the_fixed_markup() {
    assert_eq!(string("(<a><b>1</b><c/></a>).toXMLString()"), "<a>\n  <b>1</b>\n  <c/>\n</a>");
    assert_eq!(string("XML.prettyPrinting = false; (<a><b>1</b><c/></a>).toXMLString()"), "<a><b>1</b><c/></a>");
    assert_eq!(string("XML.prettyIndent = 4; (<a><b/></a>).toXMLString()"), "<a>\n    <b/>\n</a>");
    assert_eq!(string("(<a p='x&quot;&#10;'>&lt;&amp;&gt;</a>).toXMLString()"), "<a p=\"x&quot;&#xA;\">&lt;&amp;&gt;</a>");
    assert_eq!(string("(<a xmlns:n=\"u\"><n:b n:c=\"d\"/></a>).toXMLString()"), "<a xmlns:n=\"u\">\n  <n:b n:c=\"d\"/>\n</a>");
    assert_eq!(string("(<><a/><b/></>).toXMLString()"), "<a/>\n<b/>");
}

#[test]
fn embedded_expressions_are_escaped_by_position() {
    assert_eq!(string("var v = '<&>'; (<a>{v}</a>).toXMLString()"), "<a>&lt;&amp;&gt;</a>");
    assert_eq!(string("var v = 'a\"<'; (<a b={v}/>).toXMLString()"), "<a b=\"a&quot;&lt;\"/>");
    assert_eq!(string("var n = 'tag'; (<{n} x=\"1\">t</{n}>).toXMLString()"), "<tag x=\"1\">t</tag>");
    assert_eq!(string("var c = <b>1</b>; (<a>{c}</a>).toXMLString()"), "<a>\n  <b>1</b>\n</a>");
    assert_eq!(string("var a = 'p=\"1\"'; (<x {a}/>).toXMLString()"), "<x p=\"1\"/>");
    truth("var i = 0; var x = <a>{i++}{i++}</a>; x == '01' && i === 2");
    truth("var o = {t: 1}; (<a>{o.t ? 'y' : 'n'}</a>) == 'y'");
    truth("(<a>{'}'}</a>) == '}'");
    truth("(<a>{1 < 2}</a>) == 'true'");
}

#[test]
fn a_computed_tag_name_cannot_add_markup() {
    for src in [
        r#"(<{'a x="1"'}/>)"#,
        "(<{'a><c/'}></{'a'}>)",
        "(<a><b>t</{'b><c/'}></a>)",
        "(<{''}/>)",
        "(<{'1a'}/>)",
        "(<a></{'a '}>)",
        r#"(<x {'a="1" b'}="2"/>)"#,
    ] {
        assert_eq!(thrown_name(src), "SyntaxError", "{src}");
    }
}

#[test]
fn a_computed_name_may_be_any_xml_name() {
    assert_eq!(string("(<{'a.b-c'}/>).toXMLString()"), "<a.b-c/>");
    assert_eq!(string("(<{'tag'}>t</{'tag'}>).toXMLString()"), "<tag>t</tag>");
    assert_eq!(string("var n = 'n:b'; (<a xmlns:n=\"u\"><{n}/></a>).toXMLString()"), "<a xmlns:n=\"u\">\n  <n:b/>\n</a>");
}

#[test]
fn a_computed_attribute_name_matches_the_standard_example() {
    let vars = "var tagname = 'name', attributename = 'id', attributevalue = 5, content = 'Fred';";
    let literal = "<{tagname} {attributename}={attributevalue}>{content}</{tagname}>";
    assert_eq!(string(&format!("{vars} ({literal}).toXMLString()")), "<name id=\"5\">Fred</name>");
    assert_eq!(string(r#"var a = 'id'; (<x {a} = "1" b="2"/>).toXMLString()"#), "<x id=\"1\" b=\"2\"/>");
}

#[test]
fn a_computed_attribute_run_cannot_add_markup() {
    for src in [
        r#"(<x {'a="1"><y/'}></x>)"#,
        r#"(<><x {'a="1"/><y b="2"'}/></>)"#,
        "(<x {'b'}/>)",
        "(<x {'a=1'}/>)",
        r#"(<x {'a="1'}/>)"#,
        r#"(<x {'a="1"b="2"'}/>)"#,
        r#"(<x {'a="<"'}/>)"#,
    ] {
        assert_eq!(thrown_name(src), "SyntaxError", "{src}");
    }
}

#[test]
fn a_computed_attribute_run_may_hold_any_attributes() {
    truth(
        r#"var a = " a=\"1\"\n b = '>' "; var x = <x {a}/>; x.@a == '1' && x.@b == '>' && x.attributes().length() === 2 && x.children().length() === 0"#,
    );
    assert_eq!(string("(<x {''}/>).toXMLString()"), "<x/>");
    assert_eq!(string("(<x {' '} c=\"3\"/>).toXMLString()"), "<x c=\"3\"/>");
}

// ---- selectors and updates ----

#[test]
fn selectors_return_the_documented_lists() {
    let s = |rest: &str| truth(&format!("{BOOKS} {rest}"));
    s("xml.book.length() === 4 && xml.book[0].author == 'Farmer'");
    s("xml.book[0].@category == 'COOKING' && xml.book.@category.length() === 4");
    s("xml.book[1].@category.toString() === 'CHILDREN'");
    s("xml.*.length() === 4 && xml.book[0].*.length() === 3");
    s("xml.book[0].@*.length() === 1");
    s("xml.nothing.length() === 0 && xml.@nothing.length() === 0");
    s("xml..title.length() === 4 && xml..title == xml.descendants('title')");
    s("xml..@lang.length() === 4 && xml..@category.length() === 4");
    s("xml.book.(@category == 'CHILDREN').length() === 2");
    s("xml.book.(year > 1899).title.length() === 2");
    s("xml.book.(author == 'Baum').title == 'Oz'");
    s("xml.*::book.length() === 4");
    s("var n = null; xml.n::book.length() === 4");
    s("xml['book'].length() === 4 && xml.book[0]['author'] == 'Farmer' && xml.book[0]['@category'] == 'COOKING'");
    s("var k = 'category'; xml.book[0].@[k] == 'COOKING'");
}

#[test]
fn the_top_sample_has_three_children_two_elements_and_seven_descendants() {
    truth(&format!("{TOP} x.children().length() === 3"));
    truth(&format!("{TOP} x.elements().length() === 2"));
    truth(&format!("{TOP} x.descendants().length() === 7"));
}

#[test]
fn a_filter_tests_each_item_once_in_order() {
    truth(
        "var seen = []; var l = <><a>1</a><a>2</a><a>3</a></>; var r = l.(seen.push(toString()), true); seen.join() === '1,2,3' && r.length() === 3",
    );
    truth("(<a/>).(false).length() === 0");
    assert_eq!(thrown_name("({}).(1)"), "TypeError");
    assert_eq!(thrown_name("var o = {}; o.@a"), "TypeError");
    assert_eq!(thrown_name("var o = {}; o..a"), "TypeError");
}

#[test]
fn assignment_and_delete_update_the_tree() {
    let s = |rest: &str| truth(&format!("{BOOKS} {rest}"));
    s(
        "xml.book[2].author = 'Dodgeson'; xml.book[2].author.length() === 1 && xml.book[2].author == 'Dodgeson' && xml.book[2].*[1].localName() === 'author'",
    );
    s("xml.book[2].author[0] = 'First'; xml.book[2].author.length() === 3 && xml.book[2].author[0] == 'First'");
    s("xml.book[1].year = 1901; xml.book[1].year == '1901'");
    s("xml.book[1].rating = '*****'; xml.book[1].*[3].localName() === 'rating' && xml.book[1].rating == '*****'");
    s("xml.book[1].@category = 'FANTASY'; xml.book[1].@category == 'FANTASY'");
    s("xml.book[1].@added = 'y'; xml.book[1].@added == 'y' && xml.book[1].attributes().length() === 2");
    s("delete xml.book[2].author; xml.book[2].author.length() === 0");
    s("delete xml.book[2].author[1]; xml.book[2].author.length() === 2 && xml.book[2].author[1] == 'C'");
    s("delete xml.book[2].@category; xml.book[2].@category.length() === 0");
    s("xml.book[0] = <book id='new'/>; xml.book.length() === 4 && xml.book[0].@id == 'new' && xml.book[0].parent() === xml");
    assert_eq!(thrown_name("var x = <a><b/></a>; x[0] = 1"), "TypeError");
    assert_eq!(thrown_name("var x = <a><b/></a>; delete x[0]"), "TypeError");
    truth("var x = <a/>; x.b.c = 'd'; x.b.c == 'd'");
    truth("var x = <a/>; x.b += 'v'; x.b == 'v'");
    truth("var x = <a><n>1</n></a>; x.n++; x.n == '2'");
    truth("var x = <a t='1'/>; x.@t++; x.@t == '2'");
}

#[test]
fn operators_follow_the_standard() {
    truth("var l = <a/> + <b/>; typeof l === 'xml' && l.length() === 2");
    truth("var l = <><a/></> + <b/>; l.length() === 2");
    truth("typeof <a/> === 'xml' && typeof <><a/></> === 'xml' && typeof new Namespace() === 'object'");
    truth("<a><b>1</b></a> == <a><b>1</b></a> && !(<a><b>1</b></a> == <a><b>2</b></a>)");
    truth("<a p='1' q='2'/> == <a q='2' p='1'/>");
    truth("<a>1</a> == '1' && '1' == <a>1</a> && <a>1</a> == 1");
    truth("!(<a>01</a> == 1) && <a>true</a> == true && <a>null</a> == null && <a>undefined</a> == undefined && !(<a>x</a> == null)");
    truth("<><a/><b/></> == <><a/><b/></> && !(<><a/></> == <><a/><b/></>)");
    truth("new XMLList() == undefined");
    truth("new Namespace('p', 'u') == new Namespace('q', 'u') && new QName('u', 'a') == new QName('u', 'a')");
    truth("var s = ''; for (var i in <><a/><b/></>) s += i; s === '01'");
    truth("var s = ''; for each (var v in <><a>1</a><b>2</b></>) s += v; s === '12'");
    truth("var s = ''; for each (var v in [3, 4]) s += v; s === '34'");
    truth("var s = ''; for each (var v in <a>7</a>) s += v; s === '7'");
    truth("var x = <a><length>5</length></a>; x.length == '5' && x.length() === 1");
    truth("var x = <a>abc</a>; x.charAt(1) === 'b'");
    truth("var l = <><a>abc</a></>; l.toUpperCase() === 'ABC'");
    assert_eq!(thrown_name("(<a><b/></a>).noSuchMethod()"), "TypeError");
    assert_eq!(thrown_name("(<><a/><b/></>).nodeKind()"), "TypeError");
    truth("with (<a><b>1</b></a>) { b == '1' }");
}

#[test]
fn the_default_namespace_is_scoped_to_the_function() {
    truth(
        "var ns = new Namespace('u'); default xml namespace = ns; var x = <a><b/></a>; x.name().uri === 'u' && x.b.length() === 1 && x.b.name().uri === 'u'",
    );
    truth("default xml namespace = 'u'; (<a p='1'/>).@p.name().uri === ''");
    truth(
        "function f() { default xml namespace = 'u'; return (<a/>).name().uri; } \
         f() === 'u' && (<a/>).name().uri === ''",
    );
    truth("function f() { default xml namespace = 'u'; return function () { return (<a/>).name().uri; }; } f()() === 'u'");
    truth("setDefaultXMLNamespace(new Namespace('p', 'v')); (<a/>).name().uri === 'v' && new QName('b').uri === 'v'");
    truth("default xml namespace = 'u'; new QName('a').uri === 'u' && new QName('*').uri === null");
    truth("var x = <a xmlns='u'><b/></a>; x.b.length() === 0");
    truth("var x = <a xmlns='u'><b/></a>; var n = new Namespace('u'); x.n::b.length() === 1");
    truth(
        "var soap = new Namespace('http://schemas.xmlsoap.org/soap/envelope/'); default xml namespace = soap; \
         var m = <Envelope xmlns:soap='http://schemas.xmlsoap.org/soap/envelope/' soap:encodingStyle='e'><Body/></Envelope>; \
         m.@soap::encodingStyle == 'e' && m.Body.length() === 1",
    );
}

// ---- built-ins ----

#[test]
fn the_constructors_keep_identity_or_copy_as_specified() {
    truth("XML().nodeKind() === 'text' && XML(null) == '' && XML(undefined) == ''");
    truth("var x = <a/>; XML(x) === x && new XML(x) !== x && new XML(x) == x");
    truth("var l = <><a/></>; XML(l) === l[0]");
    truth("var p = <p><a q='1'><c/></a></p>; var l = p.a; var y = new XML(l); y !== l[0] && y == l[0] && y.parent() === null && l[0].parent() === p");
    truth(
        "var p = <p><a q='1'><c/></a></p>; var l = p.a; var y = new XML(l); y.@q = '2'; y.c.@r = '3'; l[0].@q == '1' && l[0].c.@r.length() === 0 && y.c.@r == '3'",
    );
    assert_eq!(thrown_name("XML(<><a/><b/></>)"), "TypeError");
    assert_eq!(thrown_name("XML('<a><b></a>')"), "SyntaxError");
    assert_eq!(thrown_name("XML('<a/><b/>')"), "SyntaxError");
    truth("XMLList().length() === 0 && XMLList('<a/><b/>').length() === 2");
    truth("var l = <><a/></>; XMLList(l) === l && new XMLList(l) !== l && new XMLList(l)[0] === l[0]");
    truth("var n = new Namespace('p', 'u'); Namespace(n) === n && new Namespace(n) !== n && new Namespace(n).prefix === 'p'");
    truth("var n = new Namespace(); n.prefix === '' && n.uri === '' && n.toString() === ''");
    truth("var n = new Namespace('u'); n.prefix === undefined && n.uri === 'u'");
    truth("new Namespace('1bad', 'u').prefix === undefined");
    assert_eq!(thrown_name("new Namespace('p', '')"), "TypeError");
    truth("var q = new QName('u', 'a'); QName(q) === q && new QName(q) !== q && q.uri === 'u' && q.localName === 'a'");
    truth("var q = new QName(new Namespace('p', 'u'), 'a'); q.uri === 'u' && q.toString() === 'u::a'");
    truth("new QName('*').toString() === '*::*' && new QName(null, 'a').uri === null && new QName().localName === ''");
    truth("isXMLName('abc') && isXMLName('a-b') && !isXMLName('a.b') && !isXMLName('$a') && !isXMLName('1a') && !isXMLName('')");
    truth("(<a.b/>).localName() === 'a.b'");
}

#[test]
fn the_settings_are_per_realm_and_restore() {
    truth(
        "var d = XML.defaultSettings(); !d.ignoreComments && !d.ignoreProcessingInstructions && d.ignoreWhitespace && d.prettyPrinting && d.prettyIndent === 2",
    );
    truth("var s = XML.settings(); XML.prettyIndent = 7; XML.settings().prettyIndent === 7 && (XML.setSettings(s), XML.prettyIndent === 2)");
    truth("XML.prettyIndent = 7; XML.setSettings(); XML.prettyIndent === 2");
    truth("XML.setSettings({ignoreWhitespace: false, prettyIndent: 'bad'}); !XML.ignoreWhitespace && XML.prettyIndent === 2");
    truth("(new XML('<a><!--c--><?p q?></a>')).children().length() === 2");
    truth("XML.ignoreComments = true; XML.ignoreProcessingInstructions = true; (new XML('<a><!--c--><?p q?></a>')).children().length() === 0");
    truth("XML.ignoreWhitespace = false; (new XML('<a> <b/> </a>')).children().length() === 3");
    truth("(new XML('<a> <b/> </a>')).children().length() === 1");
    truth("(new XML('<a> t </a>')).toString() === ' t '");
    let mut first = realm();
    first.run("XML.prettyIndent = 9").unwrap();
    assert_eq!(realm().run("XML.prettyIndent"), Ok(JsValue::Number(2.0)));
}

#[test]
fn the_xml_methods_behave_as_specified() {
    truth("var x = <a><b/></a>; x.appendChild(<c/>) === x && x.*[1].localName() === 'c' && x.c.parent() === x");
    truth("var x = <a><b/></a>; x.appendChild('t'); x.text() == 't'");
    truth("var x = <a><b/></a>; x.prependChild(<c/>); x.*[0].localName() === 'c'");
    truth("var x = <a><b/><d/></a>; x.insertChildAfter(x.b[0], <c/>); x.*[1].localName() === 'c'");
    truth("var x = <a><b/><d/></a>; x.insertChildBefore(x.d[0], <c/>); x.*[1].localName() === 'c'");
    truth("var x = <a><b/></a>; x.insertChildAfter(null, <c/>); x.*[0].localName() === 'c'");
    truth("var x = <a><b/></a>; x.insertChildBefore(null, <c/>); x.*[1].localName() === 'c'");
    truth("var x = <a><b/></a>; x.insertChildAfter(<z/>, <c/>) === undefined && x.*.length() === 1");
    truth("var x = <a><b/><c/></a>; x.c[0].childIndex() === 1 && x.childIndex() === -1");
    truth("var x = <a p='1'/>; x.attribute('p') == '1' && x.attributes().length() === 1");
    truth("var x = <a><b/>t</a>; x.child('b').length() === 1 && x.child(1) == 't' && x.child(9).length() === 0");
    truth("var x = <a><b/></a>; x.copy() !== x && x.copy() == x && x.copy().parent() === null && x.b.copy().parent() === undefined");
    truth("var c = (<a><b/></a>).b[0].copy(); c.parent() === null");
    truth("var x = <a><b/></a>; x.contains(<a><b/></a>)");
    truth("var x = <a><b/></a>; x.hasOwnProperty('b') && !x.hasOwnProperty('c') && x.hasComplexContent() && !x.hasSimpleContent()");
    truth("var x = <a>t</a>; !x.hasComplexContent() && x.hasSimpleContent() && x.length() === 1");
    truth(
        "var x = <a xmlns:p='u'><b/></a>; x.b[0].inScopeNamespaces().length === 1 && x.namespaceDeclarations()[0].prefix === 'p' && x.b[0].namespaceDeclarations().length === 0",
    );
    truth("var x = <a xmlns:p='u'/>; x.namespace('p').uri === 'u' && x.namespace('q') === undefined && x.namespace().uri === ''");
    truth("var x = <a/>; x.addNamespace(new Namespace('p', 'u')); x.namespace('p').uri === 'u'");
    truth("var x = <a xmlns:p='u'/>; x.removeNamespace(new Namespace('p', 'u')); x.namespace('p') === undefined");
    truth("var x = <a xmlns:p='u' p:q='1'/>; x.removeNamespace(new Namespace('p', 'u')); x.namespace('p').uri === 'u'");
    truth("var x = <a/>; x.name().localName === 'a' && x.localName() === 'a'");
    truth("(<a>t</a>).*[0].name() === null && (<a>t</a>).*[0].localName() === null");
    truth("var x = <a><b>1</b></a>; x.b[0].appendChild('2'); x.b[0].*.length() === 2 && (x.normalize(), x.b[0].*.length() === 1) && x.b == '12'");
    truth("var x = <a><b/></a>; x.b[0].parent() === x && x.parent() === null");
    truth("var x = <a><?p q?><?r s?></a>; x.processingInstructions().length() === 2 && x.processingInstructions('r').length() === 1");
    truth("var x = <a/>; x.propertyIsEnumerable(0) && !x.propertyIsEnumerable(1)");
    truth("var x = <a><b/><c/><b/></a>; x.replace('b', <z/>); x.*.length() === 2 && x.*[0].localName() === 'z' && x.*[1].localName() === 'c'");
    truth("var x = <a><b/></a>; x.replace(0, 't'); x.* == 't'");
    truth("var x = <a><b/></a>; x.replace('nothing', <z/>) === x && x.b.length() === 1");
    truth("var x = <a><b/></a>; x.setChildren(<c/> + <d/>) === x && x.*.length() === 2 && x.c.length() === 1");
    truth("var x = <a/>; x.setLocalName('b'); x.localName() === 'b'");
    truth("var x = <a/>; x.setName('b'); x.localName() === 'b'");
    truth("var x = <a/>; x.setNamespace(new Namespace('p', 'u')); x.name().uri === 'u' && x.toXMLString() === '<p:a xmlns:p=\"u\"/>'");
    assert_eq!(thrown_name("(<a/>).setLocalName('1 x')"), "TypeError");
    assert_eq!(thrown_name("(<a/>).setName('a b')"), "TypeError");
    assert_eq!(string("(<a>1</a>).toString()"), "1");
    assert_eq!(string("(<a><b/></a>).toString()"), "<a>\n  <b/>\n</a>");
    truth("var x = <a/>; x.valueOf() === x");
    truth("!Object.prototype.hasOwnProperty.call(XML.prototype, 'xpath') && !Object.prototype.hasOwnProperty.call(XMLList.prototype, 'xpath')");
    truth("var x = <a><b/></a>; x.b.attributes !== undefined");
}

#[test]
fn the_xml_list_methods_work_item_by_item() {
    let l = "var l = <><a p='1'><x/>s</a><a p='2'><y/><!--c--></a></>;";
    truth(&format!("{l} l.length() === 2 && l.attribute('p').length() === 2 && l.attributes().length() === 2"));
    truth(&format!("{l} l.child('x').length() === 1 && l.children().length() === 4 && l.child(0).length() === 2"));
    truth(&format!("{l} l.comments().length() === 1 && l.elements().length() === 2 && l.text() == 's'"));
    truth(&format!("{l} l.descendants().length() === 4"));
    truth(&format!("{l} l.contains(<a p='1'><x/>s</a>) && !l.contains(<z/>)"));
    truth(&format!("{l} l.copy()[0] !== l[0] && l.copy() == l"));
    truth(&format!("{l} l.hasOwnProperty(0) && !l.hasOwnProperty(5) && l.propertyIsEnumerable(1) && !l.propertyIsEnumerable(2)"));
    truth(&format!("{l} l.hasComplexContent() && !l.hasSimpleContent()"));
    truth(&format!("{l} l.parent() === undefined && l.valueOf() === l"));
    truth("var x = <r><a/><b/></r>; x.*.parent() === x");
    truth("new XMLList().hasSimpleContent() && !new XMLList().hasComplexContent() && new XMLList().parent() === undefined");
    truth("var l = <><a>1</a><a>2</a></>; l.normalize() === l && l.toString() === '<a>1</a>\\n<a>2</a>'");
    truth("var r = <r><a>1</a></r>; var l = r.a; l[0] = <a>2</a>; r.a == '2' && l[0] == '2'");
    truth("var r = <r><a>1</a></r>; r.a[1] = 'n'; r.a.length() === 2 && r.a[1] == 'n' && r.*[1].parent() === r");
    truth("var r = <r><a>1</a></r>; var list = r.b; list.c = 'v'; r.b.c == 'v'");
    truth("var r = <r><a t='1'/></r>; r.a.@t[0] = '2'; r.a.@t == '2'");
    truth("var r = <r><a>1</a><b/></r>; r.a[0] = <><p/><q/></>; r.*.length() === 3 && r.*[0].localName() === 'p'");
}

#[test]
fn inserting_a_node_moves_it_out_of_its_old_parent() {
    truth("var b = <b><c/></b>; var a = <a/>; var c = b.c[0]; a.appendChild(c); b.c.length() === 0 && a.c.length() === 1 && c.parent() === a");
    truth("var a = <a><b/><c/></a>; a.appendChild(a.b[0]); a.*.length() === 2 && a.*[0].localName() === 'c'");
    truth("var b = <b><c/></b>; var a = <a/>; var c = b.c[0]; a.appendChild(c); c.appendChild(b); a..*.length() === 2 && b.parent() === c");
}

#[test]
fn namespaces_survive_serialization() {
    assert_eq!(string("var x = <a xmlns='u'><b xmlns=''/></a>; x.toXMLString()"), "<a xmlns=\"u\">\n  <b xmlns=\"\"/>\n</a>");
    assert_eq!(string("var x = <a xml:lang='en'/>; x.toXMLString()"), "<a xml:lang=\"en\"/>");
    truth("var x = <a xmlns:p='u'><p:b/></a>; var y = new XML(x.toXMLString()); x == y && y.*[0].name().uri === 'u'");
    truth(
        "var q = new QName(new Namespace('u'), 'a'); var x = new XML('<r/>'); x.appendChild(<a/>); x.*[0].setName(q); (new XML(x.toXMLString())).*[0].name().uri === 'u'",
    );
    truth(
        "var x = <a/>; x.@[new QName('u', 'b')] = '1'; var y = new XML(x.toXMLString()); y.attributes()[0].name().uri === 'u' && y.attributes()[0] == '1'",
    );
}

const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS_NS: &str = "http://www.w3.org/2000/xmlns/";

#[test]
fn markup_cannot_declare_a_reserved_namespace_binding() {
    let declarations = [
        "xmlns:xml='wrong'".to_string(),
        "xmlns:xmlns='u'".to_string(),
        format!("xmlns:xmlns='{XMLNS_NS}'"),
        format!("xmlns:p='{XML_NS}'"),
        format!("xmlns:p='{XMLNS_NS}'"),
        format!("xmlns='{XML_NS}'"),
        format!("xmlns='{XMLNS_NS}'"),
    ];
    for declaration in declarations {
        assert_eq!(thrown_name(&format!("<a {declaration}/>")), "SyntaxError", "{declaration}");
        assert_eq!(thrown_name(&format!("new XML(\"<a {declaration}/>\")")), "SyntaxError", "{declaration}");
    }
    assert_eq!(string(&format!("(<a xmlns:xml='{XML_NS}'/>).toXMLString()")), format!("<a xmlns:xml=\"{XML_NS}\"/>"));
    assert_eq!(string("(<a xmlns=''/>).toXMLString()"), "<a xmlns=\"\"/>");
}

#[test]
fn add_namespace_rejects_a_reserved_binding_and_keeps_the_element() {
    for (prefix, uri) in [("xmlns", "u"), ("xmlns", XMLNS_NS), ("xml", "u"), ("p", XML_NS), ("p", XMLNS_NS)] {
        let src =
            format!("var x = <a/>; var name = 'none'; try {{ x.addNamespace(new Namespace('{prefix}', '{uri}')); }} catch (e) {{ name = e.name; }}");
        assert_eq!(string(&format!("{src} name")), "TypeError", "{prefix} {uri}");
        assert_eq!(string(&format!("{src} x.toXMLString()")), "<a/>", "{prefix} {uri}");
    }
    assert_eq!(
        string(&format!("var x = <a/>; x.addNamespace(new Namespace('xml', '{XML_NS}')); x.toXMLString()")),
        format!("<a xmlns:xml=\"{XML_NS}\"/>")
    );
}

#[test]
fn names_cannot_write_a_reserved_namespace_binding() {
    for setter in [
        "setNamespace(new Namespace('xmlns', 'u'))",
        "setName(new QName(new Namespace('xml', 'u'), 'b'))",
        "setNamespace(new Namespace('p', 'http://www.w3.org/2000/xmlns/'))",
        "setNamespace('http://www.w3.org/2000/xmlns/')",
    ] {
        let src = format!("var x = <a/>; var name = 'none'; try {{ x.{setter}; }} catch (e) {{ name = e.name; }}");
        assert_eq!(string(&format!("{src} name")), "TypeError", "{setter}");
        assert_eq!(string(&format!("{src} x.toXMLString()")), "<a/>", "{setter}");
    }
    let q = |prefix: &str| format!("new QName(new Namespace('{prefix}', 'u'), 'b')");
    assert_eq!(string(&format!("var x = <a/>; x[{}] = '1'; x.toXMLString()", q("xmlns"))), "<a>\n  <b xmlns=\"u\">1</b>\n</a>");
    assert_eq!(string(&format!("var x = <a/>; x.@[{}] = '1'; x.toXMLString()", q("xml"))), "<a ns0:b=\"1\" xmlns:ns0=\"u\"/>");
}

#[test]
fn an_attribute_cannot_be_renamed_xmlns() {
    for setter in ["setName('xmlns')", "setLocalName('xmlns')"] {
        let src = format!("var x = <a b='1'/>; var name = 'none'; try {{ x.@b[0].{setter}; }} catch (e) {{ name = e.name; }}");
        assert_eq!(string(&format!("{src} name")), "TypeError", "{setter}");
        assert_eq!(string(&format!("{src} x.toXMLString()")), "<a b=\"1\"/>", "{setter}");
    }
}

#[test]
fn new_nodes_cannot_take_a_reserved_name() {
    let q = format!("new QName('{XMLNS_NS}', 'b')");
    for update in ["x.@xmlns = 'u'".to_string(), format!("x[{q}] = '1'"), format!("x.@[{q}] = '1'"), format!("x[{q}][0] = '1'")] {
        let src = format!("var x = <a/>; var name = 'none'; try {{ {update}; }} catch (e) {{ name = e.name; }}");
        assert_eq!(string(&format!("{src} name")), "TypeError", "{update}");
        assert_eq!(string(&format!("{src} x.toXMLString()")), "<a/>", "{update}");
    }
}

#[test]
fn the_xmlns_uri_cannot_be_the_default_namespace() {
    assert_eq!(thrown_name(&format!("default xml namespace = '{XMLNS_NS}'")), "TypeError");
    assert_eq!(thrown_name(&format!("setDefaultXMLNamespace('{XMLNS_NS}')")), "TypeError");
}

#[test]
fn the_garbage_collector_keeps_trees_and_lists_alive() {
    let mut r = realm();
    r.run("var x = <a><b t='1'>text</b></a>; var l = x.b; var ns = new Namespace('p', 'u'); var q = new QName('u', 'n');").unwrap();
    r.collect_garbage();
    assert_eq!(r.run("x.b.@t == '1' && l[0] == 'text' && l[0].parent() === x && ns.uri === 'u' && q.localName === 'n'"), Ok(JsValue::Bool(true)));
    r.run("x = l = ns = q = null;").unwrap();
    let live = r.collect_garbage();
    assert!(live < 100_000, "{live}");
}

// ---- hostile input ----

#[test]
fn malformed_literals_are_located_syntax_errors() {
    let cases = [
        "var a = <x>",
        "<x",
        "<x></y>",
        "<x><y></x></y>",
        "<x a=></x>",
        "<x a=\"1></x>",
        "<x a b=\"1\"/>",
        "<x>&bad;</x>",
        "<x>&#xFFFFFFFFF;</x>",
        "<x>{1</x>",
        "<x>{</x>",
        "<1x/>",
        "<x a=\"<\"/>",
        "<!-- a -- b -->",
        "<x/><y/>,<",
        "<![CDATA[x",
        "<?1 x?>",
        "</x>",
        "<>",
        "<x></>",
    ];
    for src in cases {
        let e = parse(src, &e4x()).expect_err(src);
        assert!(e.line >= 1 && e.column >= 1, "{src}: {e:?}");
    }
    let e = parse("var a =\n  <x></y>;", &e4x()).unwrap_err();
    assert_eq!(e.line, 2);
}

#[test]
fn bad_xml_text_is_a_catchable_syntax_error() {
    for s in [
        "<a",
        "<a></b>",
        "<a b/>",
        "<a>&nope;</a>",
        "<a b='1' b='2'",
        "<1/>",
        "<p:a/>",
        "<a><!-- -- --></a>",
        "<!DOCTYPE a><a/>",
        "<a>]]></a>",
        "<?xml?>",
    ] {
        assert_eq!(thrown_name(&format!("new XML({s:?})")), "SyntaxError", "{s}");
    }
    assert_eq!(thrown_name("new XML('<a b=\"1\" b=\"2\"/>')"), "TypeError");
}

#[test]
fn unpaired_surrogates_are_rejected_in_xml_text() {
    let lone = "String.fromCharCode(0xD800)";
    let low = "String.fromCharCode(0xDC00)";
    for value in [
        format!("'<a>' + {lone} + '</a>'"),
        format!("'<a>' + {low} + '</a>'"),
        format!("'<a b=\"' + {lone} + '\"/>'"),
        format!("'<a><![CDATA[' + {lone} + ']]></a>'"),
        format!("'<a><!--' + {lone} + '--></a>'"),
        format!("'<a><?p ' + {lone} + '?></a>'"),
        format!("'<' + {lone} + '/>'"),
    ] {
        assert_eq!(thrown_name(&format!("new XML({value})")), "SyntaxError", "{value}");
    }
    assert_eq!(thrown_name(&format!("(<a/>).setLocalName({lone})")), "TypeError");
    let e = parse("var a = <\u{FFFD}/>;", &e4x()).unwrap_err();
    assert_eq!((e.line, e.column), (1, 10));
    truth("var p = String.fromCharCode(0xD83D, 0xDE00); var x = new XML('<a b=\"' + p + '\">' + p + '</a>'); x == p && x.@b == p");
    truth("var p = String.fromCharCode(0xD83D, 0xDE00); new XML('<' + p + '/>').localName() === p");
}

#[test]
fn collections_during_parsing_and_copying_keep_new_nodes() {
    let src = "for (var i = 0; i < 200; i++) { \
        var x = new XML('<a p=\"1\" q=\"2\"><b r=\"3\"/>t<!--c--></a>'); var c = x.copy(); \
        if (c.@p != '1' || c.b.@r != '3' || c.*.length() !== 3 || x.@q != '2') throw new Error('lost'); \
        } 'done'";
    for spare in (16_000..20_000).step_by(37) {
        let mut r = realm();
        let live = r.collect_garbage();
        r.set_limits(Limits { heap_bytes: live + spare, ..Limits::default() });
        assert_eq!(r.run(src), Ok(JsValue::String("done".into())), "{spare}");
    }
}

#[test]
fn collections_during_updates_and_queries_keep_new_values() {
    let src = "for (var i = 0; i < 60; i++) { \
        var x = new XML('<a p=\"1\" q=\"2\"><b r=\"3\"/>t<!--c--></a>'); var c = x.copy(); \
        var l = <><u v='w'/>{x.b}<y>{'t'}</y></>; \
        c.appendChild(<z k='1'/>); c.prependChild('s'); c.insertChildAfter(c.b[0], <m/>); c.insertChildBefore(c.b[0], 'n'); \
        c.replace(0, <r>1</r>); c.n.o = 'v'; c.@t = 'w'; c.z[0] = <w/>; c.g.h = 'deep'; \
        var f = x.*.(nodeKind() === 'element'); var d = x..*; var j = x + c; var m = x.b + <q/>; \
        var ns = new Namespace('p', 'u'); var qn = new QName(ns, 'a'); c.addNamespace(ns); c.setNamespace(ns); c.setName(qn); \
        var scope = c.inScopeNamespaces(); var decl = c.namespaceDeclarations(); c.normalize(); \
        var text = c.toXMLString(); var again = new XML(text); var children = c.children(); var el = c.elements(); var attrs = c.attributes(); \
        var filled = new XMLList(l); var copied = l.copy(); c.setChildren(<i/>); \
        if (x.@p != '1' || x.*.length() !== 3 || f.length() !== 1 || j.length() !== 2 || m.length() !== 2) throw new Error('lost'); \
        if (l.length() !== 3 || copied.length() !== 3 || c.*.length() !== 1 || scope.length < 1 || d.length() !== 3) throw new Error('lost'); \
        } 'done'";
    let mut first_error = None;
    let mut completed = 0;
    for spare in (40_000..56_000).step_by(211) {
        let mut r = realm();
        let live = r.collect_garbage();
        r.set_limits(Limits { heap_bytes: live + spare, ..Limits::default() });
        let got = r.run(src);
        if got == Ok(JsValue::String("done".into())) {
            completed += 1;
        } else if !matches!(&got, Err(e) if e.kind == RunErrorKind::Limit(LimitKind::Heap)) {
            first_error = Some(format!("{spare}: {got:?}"));
            break;
        }
    }
    assert_eq!(first_error, None);
    assert!(completed > 0);
}

#[test]
fn a_scope_name_skips_a_list_without_that_name() {
    truth("var outer = <o id='1'/>; var inner = <><i/></>; with (outer) with (inner) { @id == '1' }");
    truth("var outer = <o id='1'/>; var inner = <><i id='2'/></>; with (outer) with (inner) { @id == '2' }");
}

#[test]
fn bare_attribute_assignment_uses_nearest_holder() {
    truth("var x = <a b='1'/>; with (x) { @b = '2'; } x.@b == '2'");
    truth("var outer = <o id='1'/>; var inner = <i/>; with (outer) with (inner) { @id = '9'; } outer.@id == '9' && inner.@id.length() === 0");
    truth("var outer = <o id='1'/>; var inner = <i id='2'/>; with (outer) with (inner) { @id = '9'; } outer.@id == '1' && inner.@id == '9'");
    truth("var outer = <o id='1'/>; var inner = <><i/></>; with (outer) with (inner) { @id = '9'; } outer.@id == '9' && inner.@id.length() === 0");
    truth("var x = <a b='1'/>; with (x) with ({ b: 7 }) { @b = '3'; } x.@b == '3'");
    truth("with (<a b='1'/>) { (@b = 5) === 5 }");
    truth("var x = <a b='1' c='2'/>; with (x) { @b = @c = 'v'; } x.@b == 'v' && x.@c == 'v'");
    truth("var n = 0; var x = <a b='1'/>; with (x) { @[(n++, 'b')] = 'z'; } n === 1 && x.@b == 'z' && x.attributes().length() === 1");
    truth("var x = <a><c>1</c></a>; with (x) { *::c = '2'; } x.c == '2' && x.*.length() === 1");
    truth("var x = <a b='1'/>; x.@b = '4'; x.@b == '4'");
}

#[test]
fn bare_qualified_assignment_keeps_namespace() {
    truth(
        "var ns = new Namespace('u'); var x = <a xmlns:p='u'><p:c>1</p:c><c>0</c></a>; with (x) { ns::c = '2'; } \
         x.ns::c == '2' && x.ns::c.name().uri === 'u' && x.c == '0' && x.*.length() === 2",
    );
    truth(
        "var ns = new Namespace('u'); var x = <a xmlns:p='u' p:t='1' t='0'/>; with (x) { @ns::t = '5'; } \
         x.@ns::t == '5' && x.@ns::t.name().uri === 'u' && x.@t == '0'",
    );
}

#[test]
fn bare_assignment_in_filter_mutates_each_matching_item() {
    truth("var l = <><i id='1'/><i id='2'/></>; var r = l.(@id = 'z', true); r.length() === 2 && l[0].@id == 'z' && l[1].@id == 'z'");
    truth("var l = <><i n='1'/><i n='2'/></>; var r = l.(@n = 0); r.length() === 0 && l[0].@n == '0' && l[1].@n == '0'");
    truth("var l = <><i n='1'/><i n='2'/></>; var r = l.(@n = 'k'); r.length() === 2");
    truth("var o = <o k='1'/>; var l = <><i/><i/></>; with (o) { l.(@k += 'x', true); } o.@k == '1xx' && l[0].@k.length() === 0");
}

#[test]
fn bare_assignment_without_holder_throws_reference_error_before_rhs() {
    assert_eq!(thrown_name("@a = 1"), "ReferenceError");
    assert_eq!(thrown_name("with ({ a: 1 }) { @a = 1 }"), "ReferenceError");
    assert_eq!(thrown_name("with (<x/>) { @a = 1 }"), "ReferenceError");
    assert_eq!(thrown_name("with (<x/>) { @a += 1 }"), "ReferenceError");
    assert_eq!(thrown_name("with (<x/>) { @a++ }"), "ReferenceError");
    assert_eq!(thrown_name("var ns = new Namespace('u'); with (<x><c/></x>) { ns::c = 1 }"), "ReferenceError");
    truth("var n = 0; var e = ''; try { with (<x/>) { @a = (n++, 1); } } catch (err) { e = err.name; } e === 'ReferenceError' && n === 0");
    truth(
        "var b = 'g'; var x = <x/>; var e = ''; try { with (x) { @b = 'w'; } } catch (err) { e = err.name; } \
         e === 'ReferenceError' && b === 'g' && x.@b.length() === 0 && x.*.length() === 0",
    );
}

#[test]
fn bare_name_read_without_holder_throws_reference_error() {
    assert_eq!(thrown_name("with (<x/>) { @a }"), "ReferenceError");
    assert_eq!(thrown_name("var l = <><i id='1'/><i/></>; l.(@id == '1')"), "ReferenceError");
    truth("var o = <o id='0'/>; var l = <><i id='1'/><i/></>; var r; with (o) { r = l.(@id == '1'); } r.length() === 1");
}

#[test]
fn bare_scope_compound_and_updates_resolve_once() {
    truth("var n = 0; var x = <a t='1'/>; with (x) { @[(n++, 't')] += 1; } n === 1 && x.@t == '11'");
    truth("var n = 0; var x = <a t='1'/>; with (x) { @[(n++, 't')]++; } n === 1 && x.@t == '2'");
    truth("var x = <a t='1'/>; var r; with (x) { r = ++@t; } r === 2 && x.@t == '2'");
    truth("var x = <a t='1'/>; var r; with (x) { r = @t++; } r === 1 && x.@t == '2'");
    truth("var x = <a t='5'/>; var r; with (x) { r = --@t; } r === 4 && x.@t == '4'");
    truth("var x = <a t='5'/>; var r; with (x) { r = @t--; } r === 5 && x.@t == '4'");
}

#[test]
fn bare_scope_delete_removes_attribute() {
    truth("var x = <a t='1' u='2'/>; var r; with (x) { r = delete @t; } r === true && x.@t.length() === 0 && x.@u == '2'");
    truth("var x = <a><c/><d/></a>; with (x) { delete *::c; } x.c.length() === 0 && x.d.length() === 1");
    assert_eq!(thrown_name("with (<a/>) { delete @t }"), "ReferenceError");
}

#[test]
fn bare_scope_for_targets_assign_each_iteration() {
    truth("var x = <a k=''/>; var s = ''; with (x) { for (@k in { p: 1, q: 2 }) s += @k; } s === 'pq' && x.@k == 'q'");
    truth("var x = <a k=''/>; var s = ''; with (x) { for each (@k in [3, 4]) s += @k; } s === '34' && x.@k == '4'");
    assert_eq!(thrown_name("with (<a/>) { for (@k in { p: 1 }); }"), "ReferenceError");
}

#[test]
fn for_each_evaluates_its_target_before_reading_the_value() {
    truth("var o = { p: 1 }; var t = {}; for each ((o.p = 2, t).v in o); t.v === 2");
    truth("var o = { p: 1 }; var t = {}; for each (t[(o.p = 3, 'w')] in o); t.w === 3");
    truth("var x = <a k=''/>; var o = { p: 1 }; with (x) { for each (@[(o.p = 2, 'k')] in o); } x.@k == '2'");
    truth("var o = { p: 1, q: 2 }; var s = ''; for each (var v in o) s += v; var t; for each (t in o) s += t; s === '1212' && t === 2");
}

#[test]
fn bare_scope_targets_need_e4x_option() {
    for src in ["@a = 1", "ns::a = 1", "@a += 1", "++@a", "@a--", "for (@a in o);", "for each (@a in o);", "\"use strict\"; @a = 1"] {
        assert!(parse(src, &Options::default()).is_err(), "{src}");
        assert!(parse(src, &e4x()).is_ok(), "{src}");
    }
    truth("'use strict'; var l = <><i b='1'/></>; l.(@b = '2', true); l[0].@b == '2'");
}

#[test]
fn a_bracket_key_converts_to_a_string_before_it_names_a_child() {
    truth("var x = <a><true>1</true></a>; x[true] == '1' && (true in x) && !(false in x)");
    truth("var x = <a><null>1</null></a>; x[null] == '1' && x[undefined].length() === 0 && x[1.5].length() === 0 && x[-1].length() === 0");
    truth("var x = <a/>; x[false] = 'v'; x['false'] == 'v' && (delete x[false], x.*.length() === 0)");
    assert_eq!(thrown_name("(<a/>).attribute(true)"), "TypeError");
    assert_eq!(thrown_name("(<a/>).descendants(null)"), "TypeError");
}

fn kind_of(limits: Limits, src: &str) -> RunErrorKind {
    match limited(limits).run(src) {
        Ok(v) => panic!("{} completed with {v:?}", &src[..src.len().min(80)]),
        Err(e) => e.kind,
    }
}

fn on_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new().stack_size(512 << 10).spawn(f).unwrap().join().unwrap()
}

#[test]
fn deep_trees_end_in_errors_on_a_small_stack() {
    on_small_stack(|| {
        let tags = 100_000;
        let literal = format!("{}{}", "<a>".repeat(tags), "</a>".repeat(tags));
        let e = parse(&literal, &e4x()).unwrap_err();
        assert_eq!(e.message, crate::TOO_DEEP);
        let text = format!("new XML('{literal}')");
        assert_eq!(limited(Limits::default()).run(&text).map_err(|e| e.message.contains(crate::TOO_DEEP)), Err(true));
        let chain = "var x = <a/>; for (var i = 0; i < 100000; i++) { var y = <a/>; y.appendChild(x); x = y; }";
        assert_eq!(kind_of(Limits::default(), chain), RunErrorKind::Limit(LimitKind::Heap));
        let within = "var x = <a/>; for (var i = 0; i < 150; i++) { var y = <a/>; y.appendChild(x); x = y; } \
                      var c = x.copy(); [c == x, x.descendants().length(), x.toXMLString().length > 0, (x.normalize(), 1)].join()";
        assert_eq!(limited(Limits::default()).run(within), Ok(JsValue::String("true,150,true,1".into())));
    });
}

#[test]
fn cyclic_insertion_throws_and_leaves_the_tree_alone() {
    for src in [
        "x.appendChild(x)",
        "x.b[0].appendChild(x)",
        "x.b[0].c[0].appendChild(x.b[0])",
        "x.b[0].prependChild(x)",
        "x.b[0].insertChildAfter(null, x)",
        "x.b[0].insertChildBefore(null, x)",
    ] {
        let setup = "var x = <a><b><c/></b></a>; var before = x.toXMLString();";
        let script = format!("{setup} var n; try {{ {src}; n = 'none' }} catch (e) {{ n = e.name }} x.toXMLString() === before ? n : 'changed'");
        assert_eq!(string(&script), "TypeError", "{src}");
    }
}

#[test]
fn replace_inserts_a_copy_so_an_ancestor_can_be_the_value() {
    truth("var x = <a><b><c/></b></a>; x.b[0].replace(0, x); x.b.a.b.c.length() === 1 && x.b.a[0] !== x && x.descendants().length() === 4");
}

const TIGHT: Limits = Limits { fuel: 200_000, heap_bytes: 1 << 20, frames: 200, reentry: 8 };

#[test]
fn big_values_end_with_the_heap_limit() {
    let attrs = format!("var x = new XML('<a {}/>');", (0..50_000).map(|i| format!("a{i}=\"1\"")).collect::<Vec<_>>().join(" "));
    assert_eq!(kind_of(Limits { fuel: 10_000_000, ..TIGHT }, &attrs), RunErrorKind::Limit(LimitKind::Heap));
    let wide = "var l = <><a/></>; for (;;) { l = l + l; }";
    assert_eq!(kind_of(Limits { fuel: 10_000_000, ..TIGHT }, wide), RunErrorKind::Limit(LimitKind::Heap));
    let giant = "var s = 'x'; for (var i = 0; i < 40; i++) s += s; new XML('<a>' + s + '</a>')";
    assert_eq!(kind_of(Limits { fuel: 10_000_000, ..TIGHT }, giant), RunErrorKind::Limit(LimitKind::Heap));
    let escaped = "var s = '<'; for (var i = 0; i < 17; i++) s += s; (<a>{s}</a>).toXMLString().length";
    assert_eq!(kind_of(Limits { fuel: 10_000_000, ..TIGHT }, escaped), RunErrorKind::Limit(LimitKind::Heap));
}

#[test]
fn catastrophic_queries_end_with_the_fuel_limit() {
    let build = "var x = <r/>; for (var i = 0; i < 2000; i++) x.appendChild(<a/>);";
    let limits = Limits { fuel: 1_500_000, heap_bytes: 64 << 20, frames: 200, reentry: 8 };
    let filter = format!("{build} for (;;) x.a.(true);");
    assert_eq!(kind_of(limits.clone(), &filter), RunErrorKind::Limit(LimitKind::Fuel));
    let descendants = format!("{build} for (;;) x..a;");
    assert_eq!(kind_of(limits.clone(), &descendants), RunErrorKind::Limit(LimitKind::Fuel));
    let serialize = format!("{build} for (;;) x.toXMLString();");
    assert_eq!(kind_of(limits, &serialize), RunErrorKind::Limit(LimitKind::Fuel));
}

/// The fuel that `action` takes when it runs after `setup`.
fn fuel_of(setup: &str, action: &str) -> u64 {
    let mut r = realm();
    r.run(setup).unwrap();
    r.run(action).unwrap();
    Limits::default().fuel - r.fuel_left()
}

#[test]
fn insert_child_before_and_after_take_fuel_for_each_child_they_scan() {
    let narrow = "var x = <a><b/></a>; var last = x.b[0];";
    let wide = "var x = <a/>; for (var i = 0; i < 2000; i++) x.appendChild(<b/>); var last = x.b[1999];";
    for action in
        ["x.insertChildAfter(<z/>, <c/>)", "x.insertChildBefore(<z/>, <c/>)", "x.insertChildAfter(last, <c/>)", "x.insertChildBefore(last, <c/>)"]
    {
        let (n, w) = (fuel_of(narrow, action), fuel_of(wide, action));
        assert!(w >= n + 1999, "{action}: {n} with one child, {w} with 2000");
    }
}

#[test]
fn namespace_lookups_take_fuel_for_each_ancestor_they_climb() {
    let shallow = "var x = <a><a/></a>; var leaf = x.children()[0];";
    let deep = "var x = <a/>; var leaf = x; for (var i = 0; i < 150; i++) { var c = <a/>; leaf.appendChild(c); leaf = c; }";
    for action in ["leaf.inScopeNamespaces()", "leaf.namespaceDeclarations()", "leaf.namespace()", "leaf.namespace('p')"] {
        let (s, d) = (fuel_of(shallow, action), fuel_of(deep, action));
        assert!(d >= s + 149, "{action}: {s} one level down, {d} 150 levels down");
    }
}

#[test]
fn limits_cannot_be_caught_by_the_script() {
    assert_eq!(kind_of(TIGHT, "try { for (;;) (<a/>).toXMLString(); } catch (e) { 1 }"), RunErrorKind::Limit(LimitKind::Fuel));
}

/// Collects garbage and sets the heap limit to the live size plus the argument, so a script can
/// run out of heap at a chosen charge. A negative argument puts the heap over its limit, which
/// also happens when a lowered limit meets a larger heap.
fn squeeze(
    i: &mut crate::interp::Interp,
    _: crate::value::Value,
    args: &[crate::value::Value],
    _: bool,
) -> crate::interp::JsResult<crate::value::Value> {
    i.collect_garbage();
    if let Some(crate::value::Value::Number(spare)) = args.first() {
        i.limits.heap_bytes = (i.heap.bytes as f64 + *spare) as usize;
    }
    Ok(crate::value::Value::Undefined)
}

/// Runs `setup`, then the action for each step from 0 until one completes, and after each stop
/// asserts that `check` is true with the limits lifted. Returns how many runs stopped at `kind`.
fn stops_with_the_tree_intact(setup: &str, action: impl Fn(usize) -> String, check: &str, kind: LimitKind, limit: impl Fn(usize) -> Limits) -> usize {
    let mut r = realm();
    let interp = r.interp();
    let global = interp.global;
    assert!(crate::builtins::method(interp, global, "squeeze", squeeze, 1).is_ok());
    let mut stops = 0;
    for step in 0..200_000 {
        r.set_limits(Limits::default());
        r.run(setup).unwrap();
        r.set_limits(limit(step));
        let action = action(step);
        let got = r.run(&action);
        r.set_limits(Limits::default());
        assert_eq!(r.run(check), Ok(JsValue::Bool(true)), "{action}: {got:?}");
        match got {
            Ok(_) => return stops,
            Err(e) if e.kind == RunErrorKind::Limit(kind) => stops += 1,
            Err(_) => {}
        }
    }
    panic!("{check} never completed");
}

/// Runs `action`, whose `SPARE` is replaced by a call that leaves each number of bytes free in
/// turn, from 512 too few. Actions put it after the steps whose allocations a collection could
/// free: an argument is evaluated after the callee is looked up.
fn heap_stops(setup: &str, action: &str, check: &str) -> usize {
    let action = |spare: usize| action.replace("SPARE", &format!("squeeze({})", spare as i64 - 512));
    stops_with_the_tree_intact(setup, action, check, LimitKind::Heap, |_| Limits::default())
}

fn fuel_stops(setup: &str, action: &str, check: &str) -> usize {
    stops_with_the_tree_intact(setup, |_| action.to_string(), check, LimitKind::Fuel, |fuel| Limits { fuel: fuel as u64, ..Limits::default() })
}

const MOVE_SETUP: &str = "var o = <o><b/><c/></o>; var t = <t/>; var n = o.b[0];";
const MOVE_CHECK: &str =
    "o.b.length() + t.b.length() === 1 && (o.b.length() === 0 || o.b[0].parent() === o) && (t.b.length() === 0 || t.b[0].parent() === t)";

#[test]
fn a_heap_limit_during_a_move_leaves_the_node_in_its_old_parent() {
    for action in
        ["t.appendChild((SPARE, n))", "t.prependChild((SPARE, n))", "t.insertChildAfter(null, (SPARE, n))", "t.insertChildBefore(null, (SPARE, n))"]
    {
        assert!(heap_stops(MOVE_SETUP, action, MOVE_CHECK) > 0, "{action}");
    }
}

#[test]
fn a_fuel_limit_during_a_move_leaves_the_node_in_its_old_parent() {
    for action in ["t.appendChild(n)", "t.prependChild(n)"] {
        assert!(fuel_stops(MOVE_SETUP, action, MOVE_CHECK) > 0, "{action}");
    }
    let replaced = "var l = t.*; l[0] = o.b[0]";
    let setup = "var o = <o><b/><c/></o>; var t = <t><d/></t>;";
    assert!(fuel_stops(setup, replaced, MOVE_CHECK) > 0);
}

#[test]
fn a_heap_limit_on_replace_past_the_end_leaves_no_child_without_a_parent() {
    let check = "var n = x.*.length(); if (n === 0) { true } else { \
        var v = x.*[n - 1]; var name; try { v.appendChild(x); name = 'none' } catch (e) { name = e.name } v.parent() === x && name === 'TypeError' }";
    let action = "x.replace({ toString: function () { SPARE; return '5'; } }, v)";
    assert!(heap_stops("var x = <x/>; var v = <v/>;", action, check) > 0);
}

#[test]
fn limits_during_deletes_and_puts_keep_the_other_nodes() {
    let setup = "var x = <a p='1' q='2'><b>old</b><c/></a>;";
    let others = "x.c.length() === 1 && x.@q == '2'";
    for action in ["delete x.b", "delete x.@p", "x.@p = 'v'", "x.@n = 'v'", "x.b = 'v'"] {
        assert!(fuel_stops(setup, action, others) > 0, "{action}");
    }
    let held = format!("{others} && x.b.length() === 1 && (x.b == 'old' || x.b == 'v') && x.b.*[0].parent() === x.b[0]");
    assert!(fuel_stops(setup, "x.b = 'v'", &held) > 0);
    assert!(heap_stops(setup, "x.b = (SPARE, 'v')", &held) > 0);
}

/// True when the markup of `tree` is `before` or `after`, ignoring white space.
fn tree_is(tree: &str, before: &str, after: &str) -> String {
    format!("(function () {{ var s = {tree}.toXMLString().replace(/\\s/g, ''); return s === '{before}' || s === '{after}'; }})()")
}

#[test]
fn a_limit_while_a_list_replaces_a_child_leaves_the_children() {
    let setup = "var x = <a><b/><z/></a>; var v = <><p/><q/></>;";
    let replaced = tree_is("x", "<a><b/><z/></a>", "<a><p/><q/><z/></a>");
    for action in ["x.replace(0, v)", "x.b = v"] {
        assert!(fuel_stops(setup, action, &replaced) > 0, "{action}");
    }
    let spare = "x.replace({ toString: function () { SPARE; return '0'; } }, v)";
    assert!(heap_stops(setup, spare, &replaced) > 0);
    let set = tree_is("x", "<a><b/><z/></a>", "<a><p/><q/></a>");
    assert!(fuel_stops(setup, "x.setChildren(v)", &set) > 0);
}

#[test]
fn a_limit_while_a_list_is_inserted_leaves_the_children() {
    let setup = "var o = <o><p/><q/></o>; var x = <a><b/></a>; var v = o.*;";
    let check = |after: &str| {
        format!("{} && (o.*.length() === 0 ? x.*.length() === 3 : o.*.length() === 2 && x.*.length() === 1)", tree_is("x", "<a><b/></a>", after))
    };
    for (action, after) in [
        ("x.appendChild(v)", "<a><b/><p/><q/></a>"),
        ("x.prependChild(v)", "<a><p/><q/><b/></a>"),
        ("x.insertChildAfter(null, v)", "<a><p/><q/><b/></a>"),
        ("x.insertChildBefore(null, v)", "<a><b/><p/><q/></a>"),
    ] {
        assert!(fuel_stops(setup, action, &check(after)) > 0, "{action}");
    }
    assert!(heap_stops(setup, "x.appendChild((SPARE, v))", &check("<a><b/><p/><q/></a>")) > 0);
}

#[test]
fn a_heap_limit_on_a_list_put_at_an_index_keeps_the_list_and_tree_in_step() {
    let setup = "var x = <a><b/><z/></a>; var l = x.*; var v = <><p/><q/></>;";
    let check = "var s = x.toXMLString().replace(/\\s/g, ''); \
        (s === '<a><b/><z/></a>' && l.length() === 2 && l[0].name() == 'b') \
        || (s === '<a><p/><q/><z/></a>' && l.length() === 3 && l[0].name() == 'p')";
    assert!(heap_stops(setup, "l[(SPARE, 0)] = v", check) > 0);
}

#[test]
fn an_index_put_of_an_earlier_sibling_keeps_that_node_in_the_list() {
    let setup = "var x = <a><b/><c/><d/><e/></a>; var l = x.*;";
    truth(&format!("{setup} l[2] = x.b; l[2] === x.b[0] && l[3] === x.e[0]"));
    truth(&format!("{setup} l[2] = x.b[0]; l[2] === x.b[0] && l[3] === x.e[0]"));
}

/// A realm whose trees may be 32 levels deep, and a script line making `deep`, a tree of 32 levels
/// that cannot go under any element.
fn shallow_with_deep_value() -> (Realm, &'static str) {
    let deep = "var s = ''; var e = ''; for (var i = 0; i < 32; i++) { s += '<d>'; e += '</d>'; } var deep = new XML(s + e);";
    (Realm::with_options(Options { max_depth: 32, ..e4x() }, Limits::default()), deep)
}

#[test]
fn a_too_deep_value_leaves_every_match_in_place() {
    for action in ["x.replace('b', deep)", "x.b = deep"] {
        let (mut r, deep) = shallow_with_deep_value();
        r.run(deep).unwrap();
        r.run("var x = <a><b/><c/><b/></a>;").unwrap();
        assert_eq!(r.run(action).map_err(|e| e.kind), Err(RunErrorKind::Limit(LimitKind::Heap)), "{action}");
        assert_eq!(r.run("x.b.length() === 2 && x.*.length() === 3"), Ok(JsValue::Bool(true)), "{action}");
    }
}

#[test]
fn a_limit_while_matches_are_replaced_keeps_every_match() {
    let setup = "var x = <a><b/><c/><b/></a>; var n = <n/>; var l = <><p/><q/></>;";
    let two = "<a><b/><c/><b/></a>";
    for (action, after) in [
        ("x.replace('b', n)", "<a><n/><c/></a>"),
        ("x.replace('b', l)", "<a><p/><q/><c/></a>"),
        ("x.b = n", "<a><n/><c/></a>"),
        ("x.b = l", "<a><p/><q/><c/></a>"),
        ("x.b = 'v'", "<a><b>v</b><c/></a>"),
    ] {
        assert!(fuel_stops(setup, action, &tree_is("x", two, after)) > 0, "{action}");
    }
    let by_name = "x.replace({ toString: function () { SPARE; return 'b'; } }, n)";
    assert!(heap_stops(setup, by_name, &tree_is("x", two, "<a><n/><c/></a>")) > 0);
    assert!(heap_stops(setup, "x.b = (SPARE, 'v')", &tree_is("x", two, "<a><b>v</b><c/></a>")) > 0);
}

#[test]
fn matches_are_replaced_in_document_order() {
    truth("var x = <a><b>1</b><c/><b>2</b><d/><b>3</b></a>; x.replace('b', <n/>); x.*.length() === 3 && x.*[0].name() == 'n' && x.d.length() === 1");
    truth("var x = <a><b>1</b><c/><b>2</b></a>; var old = x.b[1]; x.b = 'v'; x.b.length() === 1 && x.b == 'v' && old.parent() === null");
    truth("var x = <a><c/></a>; x.b = 'v'; x.b == 'v' && x.*.length() === 2 && x.b[0].parent() === x");
}

#[test]
fn an_attribute_put_among_children_goes_in_as_text_and_stays_on_its_owner() {
    let setup = "var y = <y p='v'/>; var x = <x><b/></x>;";
    let check = "x.b.length() === 0 && x.children().length() === 1 && x.children()[0].nodeKind() === 'text' && x.toString() === 'v' \
                 && y.@p.length() === 1 && y.@p.parent() === y && y.@p[0].parent() === y";
    for edit in ["x.replace(0, y.@p)", "x.replace(0, y.@p[0])", "x.b = y.@p", "x.setChildren(y.@p)", "x.*[0] = y.@p"] {
        truth(&format!("{setup} {edit}; {check}"));
    }
    let owner = "y.@p.length() === 1 && y.@p.parent() === y && y.@p[0].parent() === y";
    for (edit, k) in [
        ("x.prependChild(y.@p)", "0"),
        ("x.prependChild(y.@p[0])", "0"),
        ("x.appendChild(y.@p)", "1"),
        ("x.appendChild(y.@p[0])", "1"),
        ("x.insertChildAfter(x.b[0], y.@p)", "1"),
        ("x.insertChildBefore(x.b[0], y.@p[0])", "0"),
    ] {
        truth(&format!(
            "{setup} {edit}; x.children().length() === 2 && x.children()[{k}].nodeKind() === 'text' && x.children()[{k}] == 'v' && {owner}"
        ));
    }
}

#[test]
fn a_list_assigned_attributes_holds_the_text_nodes_in_the_tree() {
    truth(
        "var y = <y p='v' q='w'/>; var x = <x><b/></x>; var l = x.*; l[0] = y.@*; \
         l.length() === 2 && l[0].nodeKind() === 'text' && l[0].parent() === x && l[1].nodeKind() === 'text' && l[1].parent() === x \
         && x.toString() === 'vw' && y.@*.length() === 2",
    );
}

#[test]
fn an_attribute_list_goes_in_as_text_at_the_depth_limit() {
    let open = "<a>".repeat(7);
    let close = "</a>".repeat(7);
    let src = format!(
        "var x = new XML('{open}<a/>{close}'); var c = x; while (c.a.length()) c = c.a[0]; \
         var y = <y p='v'/>; c.prependChild(y.@p); c.toString() === 'v'"
    );
    let mut realm = Realm::with_options(Options { max_depth: 8, ..e4x() }, Limits::default());
    assert_eq!(realm.run(&src), Ok(JsValue::Bool(true)));
}

#[test]
fn a_limit_during_normalize_does_not_repeat_text() {
    let setup = "var x = <a/>; x.appendChild('p'); x.appendChild('q'); x.appendChild('r');";
    let check = "x.toString() === 'pqr'";
    assert!(fuel_stops(setup, "x.normalize()", check) > 0);
}

/// Runs `action` as [`heap_stops`] does, and asserts that every run completes or stops at the heap
/// limit and that `check` is true after it. Returns how many runs stopped.
fn heap_stops_only(setup: &str, action: &str, check: &str) -> usize {
    let mut r = realm();
    let interp = r.interp();
    let global = interp.global;
    assert!(crate::builtins::method(interp, global, "squeeze", squeeze, 1).is_ok());
    let mut stops = 0;
    for spare in 0..100_000i64 {
        r.set_limits(Limits::default());
        r.run(setup).unwrap();
        let action = action.replace("SPARE", &format!("squeeze({})", spare - 512));
        let got = r.run(&action);
        r.set_limits(Limits::default());
        assert_eq!(r.run(check), Ok(JsValue::Bool(true)), "{action}: {got:?}");
        match got {
            Ok(_) => return stops,
            Err(e) if e.kind == RunErrorKind::Limit(LimitKind::Heap) => stops += 1,
            Err(e) => panic!("{action}: {e:?}"),
        }
    }
    panic!("{action} never completed");
}

#[test]
fn collections_while_a_list_is_copied_keep_every_copy() {
    let setup = "var x = <r><a/><b/></r>; var v = <c/> + <d/>; var w = undefined;";
    let set = tree_is("x", "<r><a/><b/></r>", "<r><c/><d/></r>");
    for action in ["x.setChildren((SPARE, v))", "x.* = (SPARE, v)"] {
        assert!(heap_stops_only(setup, action, &set) > 0, "{action}");
    }
    let copied = "w === undefined || (w.length() === 2 && w[0].name() == 'c' && w[1].name() == 'd' && w[0] !== v[0])";
    assert!(heap_stops_only(setup, "w = (SPARE, v).copy()", copied) > 0);
}

#[test]
fn a_limit_during_path_assignment_leaves_no_new_element() {
    let setup = "var x = <r/>; var v = <v/>;";
    for (target, value, after) in [
        ("x.a.b.c", "v", "<r><a><b><v/></b></a></r>"),
        ("x.a.b", "'text'", "<r><a><b>text</b></a></r>"),
        ("x.a.@p", "'text'", "<r><ap=\"text\"/></r>"),
        ("x.a.b[0]", "'text'", "<r><a><b>text</b></a></r>"),
    ] {
        let check = tree_is("x", "<r/>", after);
        assert!(fuel_stops(setup, &format!("{target} = {value}"), &check) > 0, "{target}");
        assert!(heap_stops_only(setup, &format!("{target} = (SPARE, {value})"), &check) > 0, "{target}");
    }
}

#[test]
fn comparing_xml_values_is_charged_to_the_heap() {
    let setup = "var x = <a p='1' q='2'><b r='3'/>t</a>; var y = x.copy();";
    assert!(heap_stops_only(setup, "(SPARE, x == y)", "x == y && y.@q == '2'") > 0);
}

/// Runs `setup` with no limit, then `action` in the same realm, and returns how `action` ended.
fn after_setup(setup: &str, action: &str) -> Result<JsValue, RunErrorKind> {
    let mut r = realm();
    let interp = r.interp();
    let global = interp.global;
    assert!(crate::builtins::method(interp, global, "squeeze", squeeze, 1).is_ok());
    r.run(setup).unwrap();
    r.run(action).map_err(|e| e.kind)
}

#[test]
fn namespace_declarations_and_qname_strings_are_charged_to_the_heap() {
    let heap = Err(RunErrorKind::Limit(LimitKind::Heap));
    let distinct = "var x = <r/>; var a = []; for (var i = 0; i < 400; i++) a.push(new Namespace('p' + i, 'u' + i));";
    for method in ["addNamespace", "setNamespace"] {
        let action = format!("squeeze(4096); for (var i = 0; i < 400; i++) x.{method}(a[i]); 1");
        assert_eq!(after_setup(distinct, &action), heap, "{method}");
    }
    let same = "var x = <r/>; var n = new Namespace('p', 'u');";
    let action = "squeeze(4096); for (var i = 0; i < 400; i++) x.addNamespace(n); 1";
    assert_eq!(after_setup(same, action), Ok(JsValue::Number(1.0)));
    let long = "var s = 'x'; for (var i = 0; i < 12; i++) s += s; var q = new QName('u', s);";
    let action = "squeeze(12000); var t1 = q.toString(); var t2 = q.toString(); 1";
    assert_eq!(after_setup(long, action), heap);
}

#[test]
fn an_interrupt_stops_serialization() {
    let mut r = realm();
    r.run("var x = <r/>; for (var i = 0; i < 500; i++) x.appendChild(<a/>);").unwrap();
    let interrupt = r.interrupt();
    r.define_function("stop", move |_| {
        interrupt.set();
        Ok(JsValue::Undefined)
    });
    assert_eq!(r.run("stop(); x.toXMLString()").map_err(|e| e.kind), Err(RunErrorKind::Limit(LimitKind::Interrupted)));
}

// ---- generated input ----

const PIECES: &[&str] = &[
    "<",
    ">",
    "/",
    "</",
    "/>",
    "<a>",
    "</a>",
    "<b c='d'>",
    "</b>",
    "<>",
    "</>",
    "{",
    "}",
    "{1}",
    "'",
    "\"",
    "&",
    "&amp;",
    "&#x41;",
    "&bad;",
    "<!--",
    "-->",
    "<![CDATA[",
    "]]>",
    "<?p",
    "?>",
    "@",
    "::",
    "..",
    ".(",
    ")",
    "*",
    "/x/",
    " ",
    "\n",
    "a",
    "1",
    "=",
    "ns",
    ":",
    "xmlns",
    "x.@a",
    "x..y",
    "default xml namespace",
];

fn tree_markup(shape: &[u8]) -> String {
    let mut out = String::new();
    let mut open: Vec<usize> = Vec::new();
    out.push_str("<r>");
    for (i, &b) in shape.iter().enumerate() {
        match b % 6 {
            0 if open.len() < 20 => {
                out.push_str(&format!("<e{i} a='{b}'>"));
                open.push(i);
            }
            1 => {
                if let Some(e) = open.pop() {
                    out.push_str(&format!("</e{e}>"));
                }
            }
            2 => out.push_str(&format!("<s{i}/>")),
            3 => out.push_str(&format!("t{b}&amp;")),
            4 => out.push_str(&format!("<!--c{b}-->")),
            _ => out.push_str(&format!("<?p{b} v?>")),
        }
    }
    while let Some(e) = open.pop() {
        out.push_str(&format!("</e{e}>"));
    }
    out.push_str("</r>");
    out
}

const TAG_PIECES: [&str; 11] = ["a", "b", " ", "=", "\"", "'", "<", "/", "&lt;", "a=\"1\"", "><y/"];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn computed_tag_parts_give_no_child_or_throw(parts in proptest::collection::vec(0..TAG_PIECES.len(), 0..5)) {
        let text: String = parts.iter().map(|&i| TAG_PIECES[i]).collect();
        let quoted = format!("{text:?}");
        for src in [
            format!("var v = <{{{quoted}}}/>; v.children().length() === 0 && v.attributes().length() === 0"),
            format!("var v = <x {{{quoted}}}></x>; v.children().length() === 0 && v.name().localName === 'x'"),
            format!("(<><x {{{quoted}}}/></>).length() === 1"),
            format!("var v = <a><b></{{{quoted}}}></a>; v.children().length() === 1"),
        ] {
            let mut r = limited(Limits { fuel: 200_000, heap_bytes: 4 << 20, frames: 200, reentry: 8 });
            match r.run(&src) {
                Ok(v) => prop_assert_eq!(v, JsValue::Bool(true), "{}", src),
                Err(e) => prop_assert!(e.kind != RunErrorKind::Internal, "{src:?}: {e:?}"),
            }
        }
    }

    #[test]
    fn source_mixes_parse_or_give_a_located_error(parts in proptest::collection::vec(0..PIECES.len(), 0..40)) {
        let src: String = parts.iter().map(|&i| PIECES[i]).collect();
        if let Err(e) = parse(&src, &e4x()) {
            prop_assert!(e.line >= 1 && e.column >= 1, "{src:?}: {e:?}");
        }
    }

    #[test]
    fn random_programs_never_reach_an_internal_error(parts in proptest::collection::vec(0..PIECES.len(), 0..40)) {
        let src: String = format!("var x = <a><y/></a>; {}", parts.iter().map(|&i| PIECES[i]).collect::<String>());
        let mut r = limited(Limits { fuel: 20_000, heap_bytes: 1 << 20, frames: 200, reentry: 8 });
        if let Err(e) = r.run(&src) {
            prop_assert!(e.kind != RunErrorKind::Internal, "{src:?}: {e:?}");
        }
    }

    #[test]
    fn xml_text_parses_or_throws(parts in proptest::collection::vec(0..PIECES.len(), 0..40)) {
        let text: String = parts.iter().map(|&i| PIECES[i]).collect();
        let quoted = format!("{text:?}");
        let mut r = limited(Limits { fuel: 200_000, heap_bytes: 4 << 20, frames: 200, reentry: 8 });
        if let Err(e) = r.run(&format!("new XML({quoted})")) {
            prop_assert!(e.kind == RunErrorKind::Thrown, "{quoted}: {e:?}");
        }
    }

    #[test]
    fn serialized_trees_parse_to_an_equal_tree(shape in proptest::collection::vec(any::<u8>(), 0..80)) {
        let markup = tree_markup(&shape);
        let quoted = format!("{markup:?}");
        let plain = format!("XML.prettyPrinting = false; XML.ignoreWhitespace = false; var x = new XML({quoted}); var y = new XML(x.toXMLString()); x.normalize(); y.normalize(); x == y");
        prop_assert_eq!(limited(Limits::default()).run(&plain), Ok(JsValue::Bool(true)), "{}", markup);
        let pretty = format!("var x = new XML({quoted}); var y = new XML(x.toXMLString()); x.toXMLString() === y.toXMLString()");
        prop_assert_eq!(limited(Limits::default()).run(&pretty), Ok(JsValue::Bool(true)), "{}", markup);
    }

    #[test]
    fn insert_sequences_stay_acyclic(steps in proptest::collection::vec((0usize..8, 0usize..8), 0..40)) {
        let mut src = String::from("var n = []; for (var i = 0; i < 8; i++) n.push(new XML('<n' + i + '/>')); var bad = 0;");
        for (a, b) in steps {
            src.push_str(&format!("try {{ n[{a}].appendChild(n[{b}]); }} catch (e) {{ if (e.name !== 'TypeError') bad++; }}"));
        }
        src.push_str("var ok = bad === 0; for (var j = 0; j < 8; j++) { ok = ok && n[j].toXMLString().length > 0 && n[j].descendants().length() < 8; } ok");
        let got = limited(Limits::default()).run(&src);
        prop_assert_eq!(got, Ok(JsValue::Bool(true)), "{}", src);
    }
}
