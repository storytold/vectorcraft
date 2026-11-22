use proptest::prelude::*;

use crate::xmp::{self, Form, Registry, XmpError};
use crate::{JsValue, Limits, Realm, RunErrorKind};

fn realm() -> Realm {
    let mut r = Realm::with_options(crate::Options::default(), Limits::default());
    if let Err(e) = r.define_external_object() {
        panic!("{e:?}");
    }
    r
}

fn run(src: &str) -> JsValue {
    match realm().run(&format!("new ExternalObject('lib:AdobeXMPScript');\n{src}")) {
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

/// The name and message of what `src` throws.
fn thrown(src: &str) -> String {
    string(&format!("try {{ {src}; 'no error' }} catch (e) {{ e.name + ': ' + e.message }}"))
}

const PACKET: &str = r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:CreatorTool="Tool &amp; Co">
   <dc:title><rdf:Alt><rdf:li xml:lang="x-default">Hello</rdf:li><rdf:li xml:lang="fr-FR">Bonjour</rdf:li></rdf:Alt></dc:title>
   <dc:subject><rdf:Bag><rdf:li>one</rdf:li><rdf:li>two</rdf:li></rdf:Bag></dc:subject>
   <dc:format>image/svg+xml</dc:format>
   <ex:thing xmlns:ex="http://example.com/ns/" rdf:parseType="Resource"><ex:a>1</ex:a></ex:thing>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>"#;

fn with_packet(src: &str) -> String {
    format!("var m = new XMPMeta({:?});\n{src}", PACKET)
}

#[test]
fn only_the_xmp_library_loads() {
    let r = realm().run("typeof XMPMeta + ' ' + ExternalObject.search('lib:AdobeXMPScript') + ' ' + ExternalObject.search('lib:other')");
    assert_eq!(r.ok(), Some(JsValue::String("undefined true false".into())));
    let r = realm().run("try { new ExternalObject('lib:other') ; 'loaded' } catch (e) { e.name + ': ' + e.message + ' ' + typeof XMPMeta }");
    assert_eq!(r.ok(), Some(JsValue::String("Error: ExternalObject: the library lib:other is not available undefined".into())));
    truth("typeof XMPMeta == 'function' && typeof XMPFile == 'function' && XMPConst.ARRAY_LAST_ITEM == -1");
    truth("var h = new ExternalObject('lib:AdobeXMPScript'); h.terminate(); typeof XMPMeta == 'function'");
}

#[test]
fn a_packet_reads_into_properties() {
    assert_eq!(string(&with_packet("m.getProperty(XMPConst.NS_DC, 'format').value")), "image/svg+xml");
    assert_eq!(string(&with_packet("String(m.getProperty(XMPConst.NS_XMP, 'xmp:CreatorTool'))")), "Tool & Co");
    assert_eq!(string(&with_packet("m.getLocalizedText(XMPConst.NS_DC, 'title', null, 'fr-FR')")), "Bonjour");
    assert_eq!(string(&with_packet("m.getLocalizedText(XMPConst.NS_DC, 'title', null, 'de-DE')")), "Hello");
    assert_eq!(string(&with_packet("m.getArrayItem(XMPConst.NS_DC, 'subject', XMPConst.ARRAY_LAST_ITEM).value")), "two");
    assert_eq!(run(&with_packet("m.countArrayItems(XMPConst.NS_DC, 'subject')")), JsValue::Number(2.0));
    assert_eq!(run(&with_packet("m.getProperty(XMPConst.NS_DC, 'title').options")), JsValue::Number(f64::from(0x200 | 0x400 | 0x800 | 0x1000)));
    assert_eq!(run(&with_packet("m.getProperty('http://example.com/ns/', 'thing').options")), JsValue::Number(256.0));
    assert_eq!(run(&with_packet("m.getProperty(XMPConst.NS_DC, 'creator')")), JsValue::Undefined);
    assert_eq!(string("XMPMeta.getNamespacePrefix('http://example.com/ns/') + ''"), "undefined");
    assert_eq!(string(&with_packet("XMPMeta.getNamespacePrefix('http://example.com/ns/')")), "ex:");
}

#[test]
fn properties_arrays_and_namespaces_round_trip() {
    let src = "var ns = 'http://example.com/rt/';
        var p = XMPMeta.registerNamespace(ns, 'rt');
        var m = new XMPMeta();
        m.setProperty(ns, 'name', 'a <b> & \"c\"');
        m.appendArrayItem(ns, 'list', 0, 'x', XMPConst.ARRAY_IS_ORDERED);
        m.appendArrayItem(ns, 'list', 'y');
        m.insertArrayItem(ns, 'list', 1, 'w');
        m.setArrayItem(ns, 'list', 3, 'z');
        m.setLocalizedText(ns, 'label', null, 'en-US', 'Hi');
        m.setLocalizedText(ns, 'label', null, 'de-DE', 'Hallo');
        var n = new XMPMeta(m.serialize());
        var o = new XMPMeta(n.serialize(XMPConst.SERIALIZE_OMIT_PACKET_WRAPPER));
        [p, o.getProperty(ns, 'rt:name').value, o.getProperty(ns, 'list').options, o.countArrayItems(ns, 'list'),
         o.getArrayItem(ns, 'list', 1).value, o.getArrayItem(ns, 'list', 2).value, o.getArrayItem(ns, 'list', 3).value,
         o.getLocalizedText(ns, 'label', null, 'x-default'), o.getLocalizedText(ns, 'label', 'de', 'de-AT'),
         m.serialize() == n.serialize(), n.serialize().indexOf('<?xpacket') == 0,
         o.serialize(XMPConst.SERIALIZE_OMIT_PACKET_WRAPPER).indexOf('<?xpacket')].join('|')";
    assert_eq!(string(src), "rt:|a <b> & \"c\"|1536|3|w|x|z|Hi|Hallo|true|true|-1");
    let src = with_packet(
        "var n = new XMPMeta(m.serialize()); n.serialize() == m.serialize() && n.getProperty('http://example.com/ns/', 'thing').options == 256 && n.getProperty(XMPConst.NS_XMP, 'CreatorTool').value == 'Tool & Co'",
    );
    truth(&src);
    truth(&with_packet(
        "m.deleteProperty(XMPConst.NS_DC, 'format'); !m.doesPropertyExist(XMPConst.NS_DC, 'format') && m.doesPropertyExist(XMPConst.NS_DC, 'subject')",
    ));
    truth(&with_packet("m.deleteArrayItem(XMPConst.NS_DC, 'subject', 1); m.getArrayItem(XMPConst.NS_DC, 'subject', 1).value == 'two'"));
    assert_eq!(string("XMPMeta.registerNamespace('http://example.com/a/', 'dc')"), "dc_1_:");
    assert_eq!(string("XMPMeta.getNamespaceURI('dc:')"), "http://purl.org/dc/elements/1.1/");
}

/// The prefix `xmlns` and the namespace of `xmlns` attributes are refused, so no packet declares
/// them.
#[test]
fn the_xmlns_prefix_and_namespace_are_refused() {
    assert_eq!(
        thrown("XMPMeta.registerNamespace('http://example.com/x/', 'xmlns')"),
        "Error: XMPMeta.registerNamespace: xmlns is not a valid property name"
    );
    assert_eq!(
        thrown("XMPMeta.registerNamespace('http://www.w3.org/2000/xmlns/', 'ok')"),
        "Error: XMPMeta.registerNamespace: ok is not a valid property name"
    );
    truth("XMPMeta.getNamespaceURI('xmlns') === undefined && XMPMeta.getNamespacePrefix('http://www.w3.org/2000/xmlns/') === undefined");
}

#[test]
fn bad_arguments_and_unbuilt_members_throw() {
    assert_eq!(thrown("new XMPMeta().getProperty('http://none/', 'a')"), "Error: XMPMeta.getProperty: the namespace http://none/ is not registered");
    assert_eq!(thrown("new XMPMeta().setProperty(XMPConst.NS_DC, '1a', 'x')"), "Error: XMPMeta.setProperty: 1a is not a valid property name");
    assert_eq!(
        thrown("new XMPMeta().setProperty(XMPConst.NS_DC, 'a', 'x\\u0001')"),
        "Error: XMPMeta.setProperty: the text holds a character XML does not allow"
    );
    assert_eq!(thrown(&with_packet("m.setProperty(XMPConst.NS_DC, 'subject', 'x')")), "Error: XMPMeta.setProperty: subject is not a simple property");
    assert_eq!(thrown(&with_packet("m.getArrayItem(XMPConst.NS_DC, 'format', 1)")), "Error: XMPMeta.getArrayItem: format is not an array");
    assert_eq!(
        thrown(&with_packet("m.insertArrayItem(XMPConst.NS_DC, 'subject', 9, 'x')")),
        "RangeError: XMPMeta.insertArrayItem: the array index 9 is out of range"
    );
    assert_eq!(
        thrown(&with_packet("m.getLocalizedText(XMPConst.NS_DC, 'subject', null, 'en')")),
        "Error: XMPMeta.getLocalizedText: subject is not a language alternative array"
    );
    assert_eq!(
        thrown(&with_packet("m.setProperty(XMPConst.NS_DC, 'subject[1]', 'x', XMPConst.PROP_IS_ARRAY)")),
        "RangeError: XMPMeta.setProperty: the value 0x200 is not available yet"
    );
    // An `rdf:Alt` with an item that has no language is not a language alternative.
    let untagged = "var m = new XMPMeta(\"<rdf:RDF xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'><rdf:Description xmlns:dc='http://purl.org/dc/elements/1.1/'>\
        <dc:title><rdf:Alt><rdf:li xml:lang='x-default'>a</rdf:li><rdf:li>b</rdf:li></rdf:Alt></dc:title></rdf:Description></rdf:RDF>\");";
    assert_eq!(
        thrown(&format!("{untagged} m.getLocalizedText(XMPConst.NS_DC, 'title', null, 'en')")),
        "Error: XMPMeta.getLocalizedText: title is not a language alternative array"
    );
    assert_eq!(
        thrown(&format!("{untagged} m.setLocalizedText(XMPConst.NS_DC, 'title', null, 'en', 'c')")),
        "Error: XMPMeta.setLocalizedText: title is not a language alternative array"
    );
    assert_eq!(
        thrown("new XMPMeta().getStructField(XMPConst.NS_DC, 'a', XMPConst.NS_DC, 'b')"),
        "Error: XMPMeta.getStructField is not available yet"
    );
    assert_eq!(thrown("new XMPMeta().getProperty(XMPConst.NS_DC, 'a/b')"), "RangeError: XMPMeta.getProperty: the value a/b is not available yet");
    assert_eq!(
        thrown("XMPMeta.prototype.getProperty.call({}, XMPConst.NS_DC, 'a')"),
        "TypeError: XMPMeta.getProperty: the object is not an XMPMeta object"
    );
    assert_eq!(
        thrown("new XMPFile('a.xmp', XMPConst.FILE_UNKNOWN, XMPConst.OPEN_FOR_READ)"),
        "Error: XMPFile without File and Folder is not available yet"
    );
}

#[test]
fn malformed_and_huge_packets_are_refused() {
    let malformed = [
        "",
        "<x:xmpmeta",
        "<a/>",
        "<!DOCTYPE x [<!ENTITY e 'boom'>]><rdf:RDF xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'/>",
        "<rdf:RDF xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'><rdf:Description>&e;</rdf:Description></rdf:RDF>",
        "<rdf:RDF xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'><rdf:Seq/></rdf:RDF>",
        "<rdf:RDF xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'><rdf:Description><nons>1</nons></rdf:Description></rdf:RDF>",
    ];
    for p in malformed {
        assert_eq!(thrown(&format!("new XMPMeta({p:?})")), "Error: XMPMeta: the XMP packet is not well-formed XMP", "{p}");
    }
    let deep = format!(
        "<rdf:RDF xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'>{}{}</rdf:RDF>",
        "<rdf:Description>".repeat(200),
        "</rdf:Description>".repeat(200)
    );
    assert_eq!(xmp::parse(&deep, &mut Registry::default()), Err(XmpError::TooLarge));
    let huge = "x".repeat(xmp::MAX_BYTES + 1);
    assert_eq!(xmp::parse(&huge, &mut Registry::default()), Err(XmpError::TooLarge));
    let r = realm().run("new ExternalObject('lib:AdobeXMPScript'); var s = 'xxxxxxxx'; while (s.length < 9000000) s += s; try { new XMPMeta(s); 'read' } catch (e) { e.name + ': ' + e.message }");
    assert_eq!(r.ok(), Some(JsValue::String("RangeError: XMPMeta: the XMP data is too large".into())));
}

#[test]
fn packets_are_found_in_file_bytes() {
    let bytes = format!("binary\0\u{1}{PACKET}trailer");
    let span = xmp::find_packet(bytes.as_bytes()).map(|s| (s.start, s.end, s.writeable));
    assert_eq!(span, Some((8, 8 + PACKET.len(), true)));
    assert_eq!(xmp::find_packet(b"<?xpacket begin='' ?> no end"), None);
    let read_only = PACKET.replace("end=\"w\"", "end=\"r\"");
    assert_eq!(xmp::find_packet(read_only.as_bytes()).map(|s| s.writeable), Some(false));
    let stray = format!("<?xpacket begin='' ?> junk <?xpacket begin='' ?>{PACKET}");
    let span = xmp::find_packet(stray.as_bytes()).map(|s| (s.start, s.end));
    assert_eq!(span, Some((stray.len() - PACKET.len(), stray.len())), "a start with no end of its own is skipped");
    let open_end = format!("<?xpacket begin='' ?> junk <?xpacket end= no close {PACKET}");
    let span = xmp::find_packet(open_end.as_bytes()).map(|s| (s.start, s.end));
    assert_eq!(span, Some((open_end.len() - PACKET.len(), open_end.len())), "a trailer that does not close before the next start is skipped");
    // A start's header must carry the wrapper id, so the same text inside a value does not start a
    // packet.
    let inner = PACKET.replace("<dc:format>image/svg+xml", "<dc:format><![CDATA[<?xpacket begin='' ?>]]>");
    let bytes = format!("binary\0{inner}");
    assert_eq!(xmp::find_packet(bytes.as_bytes()).map(|s| (s.start, s.end)), Some((7, bytes.len())));
}

/// An array whose container declares the RDF prefix itself is read as an array, also when its
/// items are not simple.
#[test]
fn a_container_declaring_its_own_prefix_is_an_array() {
    let packet = PACKET.replace(
        "<dc:format>",
        "<dc:creator><r:Seq xmlns:r=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><r:li r:parseType=\"Resource\"><dc:a>1</dc:a></r:li></r:Seq></dc:creator><dc:format>",
    );
    let o = string(&format!("String(new XMPMeta({packet:?}).getProperty(XMPConst.NS_DC, 'creator').options)"));
    let o: u32 = o.parse().unwrap();
    assert_eq!((o & xmp::PROP_IS_ARRAY, o & xmp::ARRAY_IS_ORDERED, o & xmp::PROP_IS_STRUCT), (xmp::PROP_IS_ARRAY, xmp::ARRAY_IS_ORDERED, 0));
}

#[test]
fn many_properties_and_namespaces_each_read_once() {
    let mut body = String::new();
    for n in 0..3000 {
        body.push_str(&format!("<n{}:p{n} xmlns:n{}='http://n/{}/'>v{n}</n{}:p{n}>", n % 500, n % 500, n % 500, n % 500));
    }
    body.push_str("<n0:p0 xmlns:n0='http://n/0/'>second</n0:p0>");
    let packet = format!("<rdf:RDF xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'><rdf:Description>{body}</rdf:Description></rdf:RDF>");
    let mut registry = Registry::default();
    let meta = xmp::parse(&packet, &mut registry).unwrap();
    assert_eq!(meta.props.len(), 3000);
    let first = meta.find("http://n/0/", "p0").and_then(|at| meta.props.get(at)).and_then(|p| p.value.text());
    assert_eq!(first, Some("v0"), "a property given twice keeps its first value");
    assert_eq!(registry.prefix_of("http://n/499/"), Some("n499"));
    let again = xmp::parse(&xmp::serialize(&meta, &registry, false).unwrap(), &mut registry).unwrap();
    assert_eq!(again, meta);
}

#[test]
fn blank_values_and_failed_packets_keep_their_state() {
    let rdf = "<rdf:RDF xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'><rdf:Description xmlns:a='http://a/'>\
               <a:p>  </a:p><a:q><rdf:Seq><rdf:li> \n </rdf:li></rdf:Seq></a:q></rdf:Description></rdf:RDF>";
    let src = format!(
        "new ExternalObject('lib:AdobeXMPScript'); var m = new XMPMeta({rdf:?}); var n = new XMPMeta(m.serialize());\n\
         [n.getProperty('http://a/', 'p').value, n.getArrayItem('http://a/', 'q', 1).value].join('|')"
    );
    assert_eq!(string(&src), "  | \n ");
    let bad = "<rdf:RDF xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'><rdf:Description xmlns:b='http://b/'>\
               <b:p>x</b:p><rdf:li/></rdf:Description></rdf:RDF>";
    let src = format!(
        "new ExternalObject('lib:AdobeXMPScript'); try {{ new XMPMeta({bad:?}); }} catch (e) {{}}\n\
         String(XMPMeta.getNamespacePrefix('http://b/'))"
    );
    assert_eq!(string(&src), "undefined", "a packet that fails to read registers no namespace");
}

/// A namespace URI or prefix over its limit is refused, and so is a namespace that would take the
/// realm's namespaces over their byte budget; a refused namespace leaves the registry as it was.
#[test]
fn namespace_lengths_and_the_registry_budget_are_bounded() {
    let mut registry = Registry::default();
    let uri = |n: usize, fill: usize| format!("http://n/{n}/{}", "u".repeat(fill));
    let at_limit = uri(0, xmp::MAX_URI_BYTES - uri(0, 0).len());
    assert_eq!(at_limit.len(), xmp::MAX_URI_BYTES);
    assert_eq!(registry.register(&at_limit, "a"), Ok("a".to_string()));
    let over = format!("{at_limit}u");
    assert_eq!(registry.register(&over, "b"), Err(XmpError::TooLarge));
    assert_eq!(registry.register("http://p/", &"p".repeat(xmp::MAX_PREFIX_BYTES)), Ok("p".repeat(xmp::MAX_PREFIX_BYTES)));
    assert_eq!(registry.register("http://q/", &"q".repeat(xmp::MAX_PREFIX_BYTES + 1)), Err(XmpError::TooLarge));
    assert_eq!(registry.register("http://p2/", &"p".repeat(xmp::MAX_PREFIX_BYTES)), Err(XmpError::TooLarge), "the `_1_` suffix counts");
    assert_eq!(registry.prefix_of(&over), None);
    let mut n = 1;
    let refused = loop {
        let before = registry.bytes();
        match registry.register(&uri(n, 900), &format!("p{n}")) {
            Ok(_) => n += 1,
            Err(e) => break (e, before),
        }
        assert!(n < 4096, "the byte budget ends registration before the namespace count does");
    };
    assert_eq!(refused.0, XmpError::Namespaces);
    assert_eq!(registry.bytes(), refused.1, "a refused namespace adds no bytes");
    assert!(registry.bytes() <= xmp::MAX_REGISTRY_BYTES);
    assert_eq!(registry.prefix_of(&uri(n, 900)), None);
    assert_eq!(registry.register(&at_limit, "z"), Ok("a".to_string()), "a namespace registered before keeps its prefix");
}

#[test]
fn long_namespaces_from_scripts_and_packets_are_refused() {
    assert_eq!(
        thrown("XMPMeta.registerNamespace('http://n/' + new Array(2000).join('u'), 'a')"),
        "RangeError: XMPMeta.registerNamespace: the XMP data is too large"
    );
    assert_eq!(
        thrown("XMPMeta.registerNamespace('http://n/', new Array(200).join('a'))"),
        "RangeError: XMPMeta.registerNamespace: the XMP data is too large"
    );
    let long = format!("http://n/{}", "u".repeat(xmp::MAX_URI_BYTES));
    let packet = format!(
        "<rdf:RDF xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'><rdf:Description><a:p xmlns:a='{long}'>v</a:p></rdf:Description></rdf:RDF>"
    );
    let mut registry = Registry::default();
    assert_eq!(xmp::parse(&packet, &mut registry), Err(XmpError::TooLarge));
    assert_eq!(registry.prefix_of(&long), None);
    assert_eq!(thrown(&format!("new XMPMeta({packet:?})")), "RangeError: XMPMeta: the XMP data is too large");
}

/// An error message holds at most the first 200 characters of a name, path, value or namespace
/// the script gave.
#[test]
fn error_messages_hold_a_bounded_part_of_the_script_text() {
    let cases = [
        ("new XMPMeta().getProperty('http://none/' + s, 'a')", "Error: XMPMeta.getProperty: the namespace http://none/"),
        ("new XMPMeta().setProperty(XMPConst.NS_DC, '1' + s, 'x')", "Error: XMPMeta.setProperty: 1"),
        ("new XMPMeta().getProperty(XMPConst.NS_DC, 'a/' + s)", "RangeError: XMPMeta.getProperty: the value a/"),
        ("new XMPMeta().getProperty(XMPConst.NS_DC, 'a', s)", "RangeError: XMPMeta.getProperty: the value "),
    ];
    for (src, start) in cases {
        let message = thrown(&format!("var s = new Array(100000).join('x'); {src}"));
        assert!(message.starts_with(start), "{message}");
        assert!(message.len() < start.len() + 260, "{src}: {} characters", message.len());
    }
}

/// With no item in the specific language, `setLocalizedText` changes the one item in the generic
/// language, and adds the specific language when there are several.
#[test]
fn localized_text_updates_a_single_generic_language_item() {
    let src = "var ns = 'http://example.com/lt/'; XMPMeta.registerNamespace(ns, 'lt'); var m = new XMPMeta();
        m.setLocalizedText(ns, 'label', null, 'en-GB', 'Colour');
        m.setLocalizedText(ns, 'label', 'en', 'en-US', 'Color');
        var one = [m.countArrayItems(ns, 'label'), m.getLocalizedText(ns, 'label', null, 'en-GB'), m.getLocalizedText(ns, 'label', null, 'x-default')];
        m.setLocalizedText(ns, 'label', null, 'en-AU', 'Colour');
        m.setLocalizedText(ns, 'label', 'en', 'en-CA', 'Couleur');
        one.concat([m.countArrayItems(ns, 'label'), m.getLocalizedText(ns, 'label', null, 'en-CA'), m.getLocalizedText(ns, 'label', null, 'en-GB')]).join('|')";
    assert_eq!(string(src), "2|Color|Color|4|Couleur|Color");
}

/// A `setLocalizedText` that stops at the heap limit adds no array. The heap limits tried run from
/// one that holds the text to one that also holds the change.
#[test]
fn localized_text_past_the_heap_limit_adds_no_array() {
    let mut stopped = 0;
    let mut seen = vec![];
    for step in 0..128 {
        let mut r = Realm::with_options(crate::Options::default(), Limits { heap_bytes: (256 << 10) + step * (16 << 10), ..Limits::default() });
        if let Err(e) = r.define_external_object() {
            panic!("{e:?}");
        }
        let text = "new ExternalObject('lib:AdobeXMPScript'); var m = new XMPMeta(); var s = 'x'; for (var k = 0; k < 18; k++) s += s;";
        if let Err(e) = r.run(text) {
            seen.push(format!("{step} text {e:?}"));
            continue;
        }
        if r.run("m.setLocalizedText(XMPConst.NS_DC, 'title', null, s, s);").is_ok() {
            seen.push(format!("{step} set"));
            break;
        }
        stopped += 1;
        match r.run("m.doesPropertyExist(XMPConst.NS_DC, 'title')") {
            Ok(v) => assert_eq!(v, JsValue::Bool(false), "heap step {step}"),
            Err(e) => panic!("{e:?}"),
        }
    }
    assert!(stopped > 0, "no heap limit stopped the change: {seen:?}");
}

/// Setting the `x-default` item also changes the items that had its text, and leaves the others.
#[test]
fn localized_text_for_x_default_changes_the_items_that_matched_it() {
    let src = "var ns = 'http://example.com/lt/'; XMPMeta.registerNamespace(ns, 'lt'); var m = new XMPMeta();
        m.setLocalizedText(ns, 'label', null, 'en-US', 'old');
        m.setLocalizedText(ns, 'label', null, 'fr-FR', 'vieux');
        m.setLocalizedText(ns, 'label', null, 'x-default', 'new');
        ['x-default', 'en-US', 'fr-FR'].map(function (l) { return m.getLocalizedText(ns, 'label', null, l); }).join('|')";
    assert_eq!(string(src), "new|new|vieux");
}

/// A packet read from `rdf:RDF` alone reads back from what it serializes to, which adds the
/// `x:xmpmeta` element around it, also with values nested near the depth limit.
#[test]
fn deeply_nested_values_read_back_after_serializing() {
    let mut read = 0;
    for height in 55..=66 {
        let nest = format!("{}x{}", "<a:q>".repeat(height - 1), "</a:q>".repeat(height - 1));
        let packet = format!(
            "<rdf:RDF xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'><rdf:Description xmlns:a='http://a/'><a:p>{nest}</a:p></rdf:Description></rdf:RDF>"
        );
        let mut registry = Registry::default();
        let Ok(meta) = xmp::parse(&packet, &mut registry) else { continue };
        read += 1;
        let out = xmp::serialize(&meta, &registry, true);
        let again = out.as_ref().ok().map(|out| xmp::parse(out, &mut registry));
        assert_eq!(again, Some(Ok(meta)), "a value {height} elements high");
    }
    assert!(read > 0, "some of the packets are read");
}

/// Namespaces declared only inside a structure or on a qualifier are registered when the packet
/// is read.
#[test]
fn namespaces_inside_kept_values_are_registered() {
    let packet = PACKET.replace(
        "<dc:format>",
        "<dc:source xmlns:q='http://q/' q:k='v'>s</dc:source><dc:relation rdf:parseType='Resource'><in:a xmlns:in='http://inner/'>1</in:a></dc:relation><dc:format>",
    );
    let src =
        format!("var m = new XMPMeta({packet:?}); [XMPMeta.getNamespacePrefix('http://inner/'), XMPMeta.getNamespacePrefix('http://q/')].join('|')");
    assert_eq!(string(&src), "in:|q:");
    for value in ["<a:p><bad:c>1</bad:c></a:p>", "<a:p bad:k='v'>x</a:p>"] {
        let packet = format!(
            "<rdf:RDF xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'><rdf:Description xmlns:a='http://a/'>{value}</rdf:Description></rdf:RDF>"
        );
        assert_eq!(xmp::parse(&packet, &mut Registry::default()), Err(XmpError::Malformed), "an undeclared prefix in {value}");
    }
}

/// Text beside an array's container keeps the property as it was written.
#[test]
fn text_beside_an_array_is_kept() {
    let packet = "<rdf:RDF xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'><rdf:Description xmlns:a='http://a/'>\
                  <a:p>before<rdf:Bag><rdf:li>x</rdf:li></rdf:Bag>after</a:p></rdf:Description></rdf:RDF>";
    let mut registry = Registry::default();
    let meta = xmp::parse(packet, &mut registry).unwrap();
    let out = xmp::serialize(&meta, &registry, false).unwrap();
    assert!(out.contains("before") && out.contains("after"), "{out}");
    assert_eq!(xmp::parse(&out, &mut registry), Ok(meta));
}

/// No property is set in the RDF or XML namespace, which a packet cannot hold.
#[test]
fn rdf_and_xml_namespaces_hold_no_properties() {
    for ns in ["http://www.w3.org/1999/02/22-rdf-syntax-ns#", "http://www.w3.org/XML/1998/namespace"] {
        assert_eq!(
            thrown(&format!("new XMPMeta().setProperty({ns:?}, 'foo', 'x')")),
            "Error: XMPMeta.setProperty: foo is not a valid property name",
            "{ns}"
        );
        assert_eq!(
            thrown(&format!("new XMPMeta().appendArrayItem({ns:?}, 'foo', 'x', XMPConst.PROP_IS_ARRAY)")),
            "Error: XMPMeta.appendArrayItem: foo is not a valid property name"
        );
    }
}

#[test]
fn array_forms_follow_their_options() {
    assert_eq!(Form::from_options(0x200), Form::Bag);
    assert_eq!(Form::from_options(0x400), Form::Seq);
    assert_eq!(Form::from_options(0x800), Form::Alt);
}

const PIECES: &[&str] = &[
    "<rdf:RDF xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'>",
    "</rdf:RDF>",
    "<rdf:Description xmlns:a='http://a/' rdf:about='z'>",
    "</rdf:Description>",
    "<a:p>",
    "</a:p>",
    "<rdf:Bag>",
    "</rdf:Bag>",
    "<rdf:Alt>",
    "</rdf:Alt>",
    "<rdf:li xml:lang='en'>",
    "<rdf:li>",
    "</rdf:li>",
    "<a:q rdf:resource='http://r/'/>",
    "<a:s rdf:parseType='Resource'><a:t>1</a:t></a:s>",
    "text &amp; more",
    "&bogus;",
    "<![CDATA[<x>]]>",
    "<!-- c -->",
    "<?xpacket begin=''?>",
    "<x:xmpmeta xmlns:x='adobe:ns:meta/'>",
    "</x:xmpmeta>",
    "<b:r xmlns:b='http://b/' b:k='v'>",
    "</b:r>",
];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Any arrangement of packet pieces reads without a panic, and what reads serializes to a packet
    /// that reads back the same.
    #[test]
    fn arbitrary_packets_never_panic_and_round_trip(pieces in proptest::collection::vec(0..PIECES.len(), 0..24)) {
        let text: String = pieces.iter().map(|&i| PIECES[i]).collect();
        let mut registry = Registry::default();
        if let Ok(meta) = xmp::parse(&text, &mut registry) {
            let out = xmp::serialize(&meta, &registry, true);
            prop_assert!(out.is_ok(), "{text}");
            let out = out.unwrap_or_default();
            let again = xmp::parse(&out, &mut registry);
            prop_assert_eq!(again.as_ref().ok(), Some(&meta), "{}\n{}", text, out);
        }
    }

    /// Arbitrary bytes never make the packet parser or scanner panic.
    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
        let _ = xmp::find_packet(&bytes);
        let text = String::from_utf8_lossy(&bytes);
        let _ = xmp::parse(&text, &mut Registry::default());
    }

    /// Arbitrary text through the script surface ends in a value or a script error, never an
    /// internal fault.
    #[test]
    fn arbitrary_script_values_never_fault(name in "[a-z:\\[\\]0-9/@?]{0,12}", value in "\\PC{0,16}") {
        let src = format!(
            "new ExternalObject('lib:AdobeXMPScript'); var m = new XMPMeta(); try {{ m.setProperty(XMPConst.NS_DC, {name:?}, {value:?}); \
             m.appendArrayItem(XMPConst.NS_DC, 'z', 0, {value:?}, XMPConst.PROP_IS_ARRAY); new XMPMeta(m.serialize()).getProperty(XMPConst.NS_DC, {name:?}) }} catch (e) {{ e.message }}"
        );
        if let Err(e) = realm().run(&src) {
            prop_assert!(e.kind != RunErrorKind::Internal, "{src}: {e:?}");
        }
    }
}
