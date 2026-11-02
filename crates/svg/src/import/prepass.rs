use super::convert::{MADE_UP_ID, href};
use super::layers::{INKSCAPE, marked_layer};
use super::{
    Cow, ForeignAttribute, HashMap, HashSet, ImportOptions, MAX_SVG_SCRIPTS, Node, SVG_EVENTS, XNode, css, css_transform, files, roxmltree,
    symbol_units, vector_effect,
};

/// What usvg would lose, found in the XML before it runs ([`prepass`]).
#[derive(Default)]
pub(super) struct Found {
    /// The URL of each `<a href>`'s group, by its id.
    pub(super) links: HashMap<String, String>,
    /// Ids of the undisplayed objects shown for usvg: they come back hidden.
    pub(super) hidden: HashSet<String>,
    /// The `<symbol>` id each `<use>` of a symbol shows, by the `<use>`'s id.
    pub(super) uses: HashMap<String, String>,
    /// The files `<image>` elements link to.
    pub(super) files: files::Files,
    /// Ids of the shapes whose strokes don't scale ([`vector_effect`]).
    pub(super) non_scaling: HashSet<String>,
    /// The `data-*` attributes of elements, by id ([`user_data`]).
    pub(super) data: HashMap<String, Vec<(String, String)>>,
    /// The event attributes of elements, by id ([`user_events`]).
    pub(super) events: HashMap<String, Vec<(String, String)>>,
    /// The attributes of elements in other applications' namespaces, by id ([`foreign`]).
    pub(super) foreign: HashMap<String, Vec<ForeignAttribute>>,
    /// The JavaScript files `<script>` elements link to ([`linked_scripts`]).
    pub(super) scripts: Vec<String>,
    /// Ids of the `<g>` elements marked as layers ([`marked_layer`]).
    pub(super) sublayers: HashSet<String>,
    pub(super) warnings: Vec<String>,
}

/// Changes to the SVG text: `(range, replacement)`, an empty range inserting.
#[derive(Default)]
pub(crate) struct Edits {
    list: Vec<(std::ops::Range<usize>, String)>,
    /// The ids made up so far, by element.
    ids: HashMap<roxmltree::NodeId, String>,
    /// Declarations added to elements' `style`s, by element: (where they go, whether the element
    /// has a `style` attribute, the declarations, XML-escaped).
    styles: HashMap<roxmltree::NodeId, (usize, bool, String)>,
}

impl Edits {
    fn insert(&mut self, at: usize, text: String) {
        self.list.push((at..at, text));
    }

    /// The id of element `n` (in `svg`), made up and written in when it has none.
    pub(super) fn id(&mut self, svg: &str, n: XNode) -> String {
        if let Some(id) = n.attribute("id").filter(|id| !id.is_empty()) {
            return id.to_string();
        }
        if let Some(id) = self.ids.get(&n.id()) {
            return id.clone();
        }
        let id = format!("{MADE_UP_ID}{}", self.ids.len());
        self.set(svg, n, "id", &id);
        self.ids.insert(n.id(), id.clone());
        id
    }

    /// Set attribute `name` of element `n` (in `svg`) to `value` (XML-escaped already).
    pub(crate) fn set(&mut self, svg: &str, n: XNode, name: &str, value: &str) {
        match n.attributes().find(|a| a.name() == name && a.namespace().is_none()) {
            Some(a) => self.list.push((a.range_value(), value.to_string())),
            None => self.insert(tag_name_end(svg, n), format!(" {name}=\"{value}\"")),
        }
    }

    /// Add the declaration `decl` (XML-escaped already) to the `style` of element `n` (in `svg`),
    /// after the ones already there (so it wins over them and over style sheet rules).
    fn style(&mut self, svg: &str, n: XNode, decl: &str) {
        let (_, _, decls) =
            self.styles.entry(n.id()).or_insert_with(|| match n.attributes().find(|a| a.name() == "style" && a.namespace().is_none()) {
                Some(a) => (a.range_value().end, true, String::new()),
                None => (tag_name_end(svg, n), false, String::new()),
            });
        decls.push(';');
        decls.push_str(decl);
    }

    /// `svg` with the edits made; overlapping ones and ranges off character boundaries are
    /// skipped.
    fn apply(mut self, svg: &str) -> Cow<'_, str> {
        for (at, has_style, decls) in std::mem::take(&mut self.styles).into_values() {
            self.insert(at, if has_style { decls } else { format!(" style=\"{}\"", decls.strip_prefix(';').unwrap_or(&decls)) });
        }
        if self.list.is_empty() {
            return svg.into();
        }
        // Stable: inserts at one place keep their order.
        self.list.sort_by_key(|(r, _)| (r.start, r.end));
        let mut out = String::with_capacity(svg.len() + self.list.iter().map(|(_, t)| t.len()).sum::<usize>());
        let mut last = 0;
        for (r, text) in self.list {
            let (Some(part), Some(_)) = (svg.get(last..r.start), svg.get(r.clone())) else { continue };
            out.push_str(part);
            out.push_str(&text);
            last = r.end;
        }
        out.push_str(svg.get(last..).unwrap_or(""));
        out.into()
    }
}

/// usvg reads `<a href>` and `<use>` as plain groups, drops what isn't displayed and reads linked
/// files on its own. Before it runs, the SVG is changed so that what we keep can be found:
/// * every `<a href>` and every `<use>` of a `<symbol>` gets an id when it has none
///   ([`MADE_UP_ID`]…), recorded with its URL or symbol;
/// * undisplayed objects (`display: none`, as Save writes hidden layers) are shown and recorded,
///   to come back hidden; one something links to (a `<use>` template) stays as it is;
/// * shapes with a non-scaling stroke get an id when they have none, recorded ([`vector_effect`]);
/// * objects with event attributes get an id when they have none, recorded with them
///   ([`user_events`]), and the `<script>` links are recorded ([`linked_scripts`]);
/// * so do objects with attributes in other applications' namespaces ([`foreign`]);
/// * linked files are read ([`files`]);
/// * percentage lengths in a `<symbol>` with a `viewBox` become numbers ([`symbol_units`]);
/// * what style sheet rules with a `:not()` set goes in `style` attributes ([`not_rules`]).
///
/// Whether each is there at all is read from `markers`: `svg` with its data URLs left out
/// ([`depth::prescan`]), so an embedded picture isn't searched a dozen times. The edits are made
/// in `svg` itself.
pub(super) fn prepass<'s>(svg: &'s str, markers: &str, xml: &roxmltree::Document, opts: &ImportOptions) -> (Cow<'s, str>, Found) {
    let mut found = Found::default();
    // (`:` for prefixed elements such as `<svg:image>`.)
    let css_transform = has_css_transform(markers);
    let not = markers.contains(":not(");
    let events = SVG_EVENTS.iter().any(|e| markers.contains(e));
    let foreign_attrs = markers.contains("xmlns:") && has_foreign_attributes(xml);
    if !css_transform
        && !not
        && !events
        && !foreign_attrs
        && !["<a", "display", "<use", ":use", "<image", ":image", vector_effect::NON_SCALING, "data-", "groupmode", "script"]
            .iter()
            .any(|t| markers.contains(t))
    {
        return (svg.into(), found);
    }
    let mut edits = Edits::default();
    let elements = || xml.descendants().filter(|n| n.is_element());
    for a in elements().filter(|n| n.tag_name().name() == "a") {
        if let Some(url) = href(a) {
            found.links.insert(edits.id(svg, a), url.to_string());
        }
    }
    if markers.contains("data-") {
        for e in elements().filter(|e| OBJECT_ELEMENTS.contains(&e.tag_name().name())) {
            let data = user_data(e);
            if !data.is_empty() {
                found.data.insert(edits.id(svg, e), data);
            }
        }
    }
    if foreign_attrs {
        for e in elements().filter(|e| OBJECT_ELEMENTS.contains(&e.tag_name().name())) {
            let kept = foreign(e, &mut found.warnings);
            if !kept.is_empty() {
                found.foreign.insert(edits.id(svg, e), kept);
            }
        }
    }
    if events {
        for e in elements().filter(|e| EVENT_ELEMENTS.contains(&e.tag_name().name())) {
            let events = user_events(e);
            if !events.is_empty() {
                found.events.insert(edits.id(svg, e), events);
            }
        }
    }
    if markers.contains("script") {
        found.scripts = linked_scripts(xml, &mut found.warnings);
    }
    found.sublayers = elements().filter(|g| marked_layer(*g)).filter_map(|g| g.attribute("id")).map(str::to_string).collect();
    let symbols: HashSet<&str> = elements().filter(|n| n.tag_name().name() == "symbol").filter_map(|n| n.attribute("id")).collect();
    for u in elements().filter(|n| n.tag_name().name() == "use") {
        if let Some(sym) = href(u).and_then(|h| h.strip_prefix('#')).filter(|s| symbols.contains(s)) {
            found.uses.insert(edits.id(svg, u), sym.to_string());
        }
    }
    let (display, non_scaling) = (markers.contains("display"), markers.contains(vector_effect::NON_SCALING));
    if display || non_scaling || not {
        let css = css::Styles::new(xml);
        // First: an undisplayed object shown for usvg is shown after these.
        if not && css.has_not_rules() {
            not_rules(svg, xml, &css, &mut edits);
        }
        if display {
            hidden_objects(svg, xml, &css, &mut edits, &mut found.hidden);
        }
        if non_scaling {
            vector_effect::find(svg, xml, &css, &mut edits, &mut found.non_scaling, &mut found.warnings);
        }
    }
    found.files = files::Files::read(svg, xml, opts, &mut edits, &mut found.warnings);
    if !symbols.is_empty() && markers.contains('%') {
        edits.list.extend(symbol_units::rewrites(xml));
    }
    if css_transform {
        edits.list.extend(css_transform::rewrites(svg, xml));
    }
    (edits.apply(svg), found)
}

/// Is there a `transform` CSS declaration in `markers` (`transform` followed by `:`)?
fn has_css_transform(markers: &str) -> bool {
    markers.match_indices("transform").any(|(i, t)| markers.get(i + t.len()..).is_some_and(|s| s.trim_start().starts_with(':')))
}

/// usvg's style sheets drop rules with a `:not()` selector (#1557). The properties such rules set
/// on an element go in its `style` attribute, with the value that wins the cascade there (which
/// can be another rule's or the attribute's own), so usvg reads what CSS would.
fn not_rules(svg: &str, xml: &roxmltree::Document, css: &css::Styles, edits: &mut Edits) {
    for n in xml.descendants().filter(XNode::is_element) {
        for (name, value) in css.set_by_not_rules(n) {
            let value = if name == "transform" { css_transform::to_svg(&value).unwrap_or(value) } else { value };
            edits.style(svg, n, &crate::xml_escape(&format!("{name}:{value}")));
        }
    }
}

/// The elements that are objects (what [`hidden_objects`] shows).
const OBJECTS: &[&str] = &["g", "a", "path", "rect", "circle", "ellipse", "line", "polyline", "polygon", "image", "text", "use"];

/// Show the undisplayed objects outside `<defs>` (and the like) for usvg, recording their ids. The
/// objects in a `<switch>` count too (some apps wrap all their layers in one).
fn hidden_objects(svg: &str, xml: &roxmltree::Document, css: &css::Styles, edits: &mut Edits, ids: &mut HashSet<String>) {
    let linked: HashSet<&str> = xml.descendants().filter_map(href).filter_map(|h| h.strip_prefix('#')).collect();
    let mut todo: Vec<XNode> = xml.root_element().children().filter(XNode::is_element).collect();
    while let Some(n) = todo.pop() {
        let tag = n.tag_name().name();
        if n.attribute("id").is_some_and(|id| linked.contains(id)) {
            continue;
        }
        if matches!(tag, "g" | "a" | "switch") {
            todo.extend(n.children().filter(XNode::is_element));
        }
        if !OBJECTS.contains(&tag) || css.own(n, "display").as_deref() != Some("none") {
            continue;
        }
        ids.insert(edits.id(svg, n));
        // A `style` declaration wins over the attribute and style sheet rules.
        edits.style(svg, n, "display:inline");
    }
}

/// Where the start tag of element `n` ends its name (where an attribute can go).
fn tag_name_end(svg: &str, n: XNode) -> usize {
    let start = n.range().start;
    svg.get(start + 1..).and_then(|s| s.find(|c: char| c.is_whitespace() || c == '/' || c == '>')).map_or(start + 2, |i| start + 1 + i)
}

/// The elements that become objects, whose `data-*` attributes are kept.
const OBJECT_ELEMENTS: &[&str] = &["g", "a", "use", "path", "rect", "circle", "ellipse", "line", "polyline", "polygon", "image"];

/// The most `data-*` attributes kept per element (the file is untrusted).
const MAX_DATA: usize = 64;

/// The `data-*` attributes of element `e`, in order, without the `data-` (ours, `data-name` and
/// `data-vc-*`, aside).
fn user_data(e: XNode) -> Vec<(String, String)> {
    e.attributes()
        .filter(|a| a.namespace().is_none())
        .filter_map(|a| Some((a.name().strip_prefix("data-")?, a.value())))
        .filter(|(k, _)| !k.is_empty() && *k != "name" && !k.starts_with("vc-"))
        .take(MAX_DATA)
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// The namespaces whose attributes VectorCraft reads, writes or makes again itself: their
/// attributes aren't kept as another application's ([`foreign`]).
const OWN_NAMESPACES: &[&str] = &[
    "http://www.w3.org/2000/svg",
    "http://www.w3.org/1999/xlink",
    "http://www.w3.org/XML/1998/namespace",
    "http://www.w3.org/2000/xmlns/",
    INKSCAPE,
    "http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd",
    crate::EDITING_NS,
];

/// The most attributes in other applications' namespaces kept per element (the file is untrusted).
pub const MAX_FOREIGN: usize = 64;

/// The longest value of such an attribute kept, in bytes; a longer one is left out.
pub const MAX_FOREIGN_VALUE: usize = 64 * 1024;

/// The namespace of attribute `a` when it is another application's ([`OWN_NAMESPACES`] aside).
fn foreign_ns<'a>(a: &roxmltree::Attribute<'a, '_>) -> Option<&'a str> {
    a.namespace().filter(|ns| !OWN_NAMESPACES.contains(ns))
}

/// Does any element of `xml` have an attribute in another application's namespace? Most files
/// declare only namespaces VectorCraft reads itself (XLink), and skip the scan for them.
fn has_foreign_attributes(xml: &roxmltree::Document) -> bool {
    xml.descendants().filter(XNode::is_element).any(|e| e.attributes().any(|a| foreign_ns(&a).is_some()))
}

/// The attributes of element `e` in other applications' namespaces, in order, with the prefix the
/// file gives each namespace: `plotter:pen="2"`. Those past [`MAX_FOREIGN`], and values longer
/// than [`MAX_FOREIGN_VALUE`], are left out with a warning.
fn foreign(e: XNode, warnings: &mut Vec<String>) -> Vec<ForeignAttribute> {
    let mut kept = vec![];
    let mut left_out = false;
    for a in e.attributes() {
        let Some(ns) = foreign_ns(&a) else { continue };
        if kept.len() == MAX_FOREIGN || a.value().len() > MAX_FOREIGN_VALUE {
            left_out = true;
            continue;
        }
        // A namespaced attribute always has a prefix in the file; `ns` stands in where the only
        // one found is the default namespace's (export makes it unique).
        let prefix = e.lookup_prefix(ns).filter(|p| !p.is_empty()).unwrap_or("ns");
        kept.push(ForeignAttribute { ns: ns.to_string(), prefix: prefix.to_string(), name: a.name().to_string(), value: a.value().to_string() });
    }
    if left_out {
        let w = format!(
            "other applications' attributes past {MAX_FOREIGN} on an element, or longer than {} KiB, were left out",
            MAX_FOREIGN_VALUE / 1024
        );
        if !warnings.contains(&w) {
            warnings.push(w);
        }
    }
    kept
}

/// The elements whose event attributes are kept: the objects, and type.
const EVENT_ELEMENTS: &[&str] = &["g", "a", "use", "path", "rect", "circle", "ellipse", "line", "polyline", "polygon", "image", "text"];

/// The event attributes of element `e` that the SVG Interactivity panel lists ([`SVG_EVENTS`]),
/// in order: the JavaScript is kept as text and never run.
fn user_events(e: XNode) -> Vec<(String, String)> {
    e.attributes()
        .filter(|a| a.namespace().is_none() && SVG_EVENTS.contains(&a.name()))
        .map(|a| (a.name().to_string(), a.value().to_string()))
        .collect()
}

/// The JavaScript files the `<script>` elements link to, in order and each once, at most
/// [`MAX_SVG_SCRIPTS`] (the file is untrusted): the links are kept as text, and the files are never
/// read or run. Inline scripts and the links past the limit are left out with a warning.
fn linked_scripts(xml: &roxmltree::Document, warnings: &mut Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let mut over = false;
    for s in xml.descendants().filter(|n| n.is_element() && n.tag_name().name() == "script") {
        match href(s) {
            Some(h) if out.iter().any(|o| o == h) => {}
            Some(h) if out.len() < MAX_SVG_SCRIPTS => out.push(h.to_string()),
            Some(_) => over = true,
            None if s.text().is_some_and(|t| !t.trim().is_empty()) => {
                let w = "inline scripts were left out: only linked JavaScript files are kept".to_string();
                if !warnings.contains(&w) {
                    warnings.push(w);
                }
            }
            None => {}
        }
    }
    if over {
        warnings.push(format!("more than {MAX_SVG_SCRIPTS} JavaScript files are linked: only the first {MAX_SVG_SCRIPTS} were kept"));
    }
    out
}

/// Make `n` link to `url`, unless it links somewhere already (an inner link wins).
pub(super) fn link(n: &mut Node, url: &str) {
    if n.url().is_none() {
        n.edit_attrs(|a| a.url = url.to_string());
    }
}

/// The names apps keep beside element ids, by id: `data-name`, `inkscape:label`, `serif:id` or
/// `aria-label` (the first one present).
pub(super) fn labels(xml: &roxmltree::Document) -> HashMap<String, String> {
    const SERIF: &str = "http://www.serif.com/";
    xml.descendants()
        .filter_map(|n| {
            let id = n.attribute("id").filter(|id| !id.is_empty())?;
            let name = n
                .attribute("data-name")
                .or_else(|| n.attribute((INKSCAPE, "label")))
                .or_else(|| n.attribute((SERIF, "id")))
                .or_else(|| n.attribute("aria-label"))?;
            Some((id.to_string(), name.to_string()))
        })
        .collect()
}
