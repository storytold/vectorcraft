//! Hand-written Illustrator editing data (the format `vectorcraft_eps`'s reader documents) and the
//! EPS, `.ai` and PDF files that carry it, for import tests. Nothing here comes from files another
//! app wrote, except the key names that [`pdf`] uses, the XML node keys of [`xml_node`] and
//! the keys of [`on_path_record`] and [`on_path_member`].

/// Editing data of a `width` × `height` page: header comments (the art's bounding box and the crop
/// marks are the page), `body` (setup and layers), the trailer.
pub fn editing_data(width: f64, height: f64, body: &str) -> String {
    editing_data_in([0.0, 0.0, width, height], [0.0, 0.0, width, height], body)
}

/// [`editing_data`] whose art's bounding box (`x0 y0 x1 y1`, art space) is `art` and whose
/// artboard (the crop marks) is `board`.
pub fn editing_data_in(art: [f64; 4], board: [f64; 4], body: &str) -> String {
    let [a0, a1, a2, a3] = art;
    let [b0, b1, b2, b3] = board;
    format!(
        "{PS_HEADER}\n%%Creator: VectorCraft tests\n%%BoundingBox: {a0} {a1} {a2} {a3}\n%%HiResBoundingBox: {a0} {a1} {a2} {a3}\n\
         %AI5_FileFormat 14.0\n%AI3_Cropmarks: {b0} {b1} {b2} {b3}\n%%EndComments\n\
         %%BeginProlog\n%%EndProlog\n%%BeginSetup\n%%EndSetup\n{body}\n%%PageTrailer\ngsave annotatepage grestore showpage\n%%Trailer\n%%EOF\n"
    )
}

/// The first line of a PostScript file (the DSC version comment).
const PS_HEADER: &str = "%!PS-Adobe-3.0"; // brand-ok: the PostScript header comment
/// The `PieceInfo` key the private data streams are under.
const PIECE: &str = "Illustrator"; // brand-ok: the key the files use

/// A layer named `name` (shown, unlocked, printing, light blue) holding `art`.
pub fn layer(name: &str, art: &str) -> String {
    format!("%AI5_BeginLayer\n1 1 1 1 0 0 1 0 79 128 255 0 50 0 Lb\n({name}) Ln\n0 A\n0 Xw\n{art}\nLB\n%AI5_EndLayer--\n")
}

/// The path of the rectangle `x0 y0 x1 y1` (art space: y up).
pub fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> String {
    format!("{x0} {y0} m\n{x0} {y1} L\n{x1} {y1} L\n{x1} {y0} L\n{x0} {y0} L\n")
}

/// `name` given to the object before it (its XML id).
pub fn named(name: &str) -> String {
    format!("%_/ArtDictionary :\n%_/XMLUID : ({}) ; (AI10_ArtUID) ,\n%_;\n%_\n", name.replace(' ', "_"))
}

/// The value of an XML node of the document data ([`xml_node`]), with the type it is written with.
#[derive(Clone, Copy, Debug)]
pub enum XmlValue<'a> {
    /// A `/String`.
    Str(&'a str),
    /// A `/UnicodeString`.
    Unicode(&'a str),
    /// A `/Bool`, `0` or `1`.
    Bool(bool),
    /// Anything else, written as it is before the key (`5 /Int`).
    Raw(&'a str),
}

/// `s` as PostScript string literals on `%_` lines: `\`, `(` and `)` escaped, and a string longer
/// than 40 bytes split into literals of at most 40 bytes on lines of their own, the last one left
/// open for what follows it on its line.
fn ps_strings(s: &str) -> String {
    let escaped = |part: &str| part.replace('\\', "\\\\").replace('(', "\\(").replace(')', "\\)");
    let mut parts = vec![];
    let mut start = 0;
    for (at, c) in s.char_indices() {
        if at + c.len_utf8() - start > 40 {
            parts.push(&s[start..at]);
            start = at;
        }
    }
    parts.push(&s[start..]);
    let Some((last, first)) = parts.split_last() else { return String::new() };
    let lines: String = first.iter().map(|p| format!("%_({})\n", escaped(p))).collect();
    format!("{lines}%_({})", escaped(last))
}

/// `value` written before a key on a `%_` line, the line's `%_` included.
fn xml_value(value: XmlValue) -> String {
    match value {
        XmlValue::Str("") => "%_ /String".into(),
        XmlValue::Str(s) => format!("{} /String", ps_strings(s)),
        XmlValue::Unicode(s) => format!("{} /UnicodeString", ps_strings(s)),
        XmlValue::Bool(b) => format!("%_{} /Bool", u8::from(b)),
        XmlValue::Raw(r) => format!("%_{r}"),
    }
}

/// An XML node of the document data (`/XMLNode`): its name, its value, its type `kind` (1 an
/// element, 2 an attribute, 3 text, 9 the document), its children (an array) and its attribute
/// nodes (a dictionary, each under its name).
pub fn xml_node(kind: u8, name: &str, value: XmlValue, attributes: &[(&str, XmlValue)], children: &[String]) -> String {
    let attributes: String = attributes.iter().map(|(k, v)| format!("{}%_; ({k}) ,\n", xml_node_open(2, k, *v, &[], ""))).collect();
    format!("{}%_;\n", xml_node_open(kind, name, value, children, &attributes))
}

/// [`xml_node`] up to the `;` that closes it.
fn xml_node_open(kind: u8, name: &str, value: XmlValue, children: &[String], attributes: &str) -> String {
    format!(
        "%_/XMLNode :\n{} (xmlnode-nodename) ,\n{} (xmlnode-nodevalue) ,\n%_{kind} /Int (xmlnode-nodetype) ,\n\
         %_/Array :\n{}%_; (xmlnode-children) ,\n%_/Dictionary :\n{attributes}%_; (xmlnode-attributes) ,\n",
        xml_value(XmlValue::Str(name)),
        xml_value(value),
        // Each child is closed as an item of the array.
        children.iter().map(|c| format!("{}%_; ,\n", c.strip_suffix("%_;\n").unwrap_or(c))).collect::<String>()
    )
}

/// An element `name` with the attributes `attributes` (each a `/UnicodeString`) and the children
/// `children`.
pub fn xml_element(name: &str, attributes: &[(&str, &str)], children: &[String]) -> String {
    let attributes: Vec<(&str, XmlValue)> = attributes.iter().map(|(k, v)| (*k, XmlValue::Unicode(v))).collect();
    xml_node(1, name, XmlValue::Str(""), &attributes, children)
}

/// A text node (`#text`) holding `value`.
pub fn xml_text(value: XmlValue) -> String {
    xml_node(3, "#text", value, &[], &[])
}

/// The document data with the XML nodes `library` (a `variableSets` element): a `/Document`
/// dictionary whose `Recorded` dictionary holds them as the XML document `#document`, in its
/// `metadata` element.
pub fn variable_library_data(library: &str) -> String {
    let document = xml_node(9, "#document", XmlValue::Str(""), &[], &[xml_element("metadata", &[], &[library.to_string()])]);
    let document = document.strip_suffix("%_;\n").unwrap_or(&document);
    format!("%AI9_BeginDocumentData\n%_/Document :\n%_/Dictionary :\n{document}%_; (#document) ,\n%_; /Recorded ,\n%_;\n%AI9_EndDocumentData\n")
}

/// The document data with a variable library ([`variable_library_data`]) of one variable set,
/// unlocked, with the text variables `variables` and the data sets `data_sets`, each with a `p`
/// element for every variable (`<data set> <variable>`).
pub fn variable_library(variables: &[&str], data_sets: &[&str]) -> String {
    let declared: Vec<String> = variables.iter().map(|v| xml_element("variable", &[("trait", "textcontent"), ("varName", v)], &[])).collect();
    let sets: Vec<String> = data_sets
        .iter()
        .map(|d| {
            let values: Vec<String> = variables
                .iter()
                .map(|v| xml_element(v, &[], &[xml_element("p", &[], &[xml_text(XmlValue::Unicode(&format!("{d} {v}")))])]))
                .collect();
            xml_element("v:sampleDataSet", &[("dataSetName", d)], &values)
        })
        .collect();
    let set = xml_element(
        "variableSet",
        &[("locked", "none"), ("varSetName", "binding1")],
        &[xml_element("variables", &[], &declared), xml_element("v:sampleDataSets", &[("xmlns", "&ns_custom;"), ("xmlns:v", "&ns_vars;")], &sets)],
    );
    variable_library_data(&xml_element("variableSets", &[("xmlns", "&ns_vars;")], &[set]))
}

/// The linked file of [`sample_variable_sets`]' second data set: a path that names no file.
pub const SAMPLE_MISSING_FILE: &str = "/Users/test/Missing Folder (old)/a linked file that isn't there.eps";

/// A `variableSets` element for [`variable_library_data`]: one variable set, its `locked` attribute
/// `locked`, with the variables `v_text` (text), `v_visible` (visibility), `v_file` (a linked file),
/// `v_graph` (a graph) and `v_unbound` (no kind), and the data sets `ds_one`, `ds_two` (the one
/// shown) and `ds_three`. `ds_two`'s text is two paragraphs, `line one` and `line two & < é`, its
/// visibility false, its linked file [`SAMPLE_MISSING_FILE`] and `v_unbound` empty; the elements
/// `more` follow its values. No data set has a graph value.
pub fn sample_variable_sets(locked: &str, more: &[String]) -> String {
    let declare = |name: &str, kind: Option<(&str, &str)>, art: Option<&str>| {
        let mut attributes = vec![];
        if let Some((t, category)) = kind {
            attributes.push(("trait", XmlValue::Str(t)));
            attributes.push(("category", XmlValue::Str(category)));
        }
        attributes.push(("varName", XmlValue::Unicode(name)));
        let art = art.map(|a| format!("/XMLUIDREF : ({a}) ;"));
        if let Some(a) = &art {
            attributes.push(("docRef", XmlValue::Raw(a)));
        }
        xml_node(1, "variable", XmlValue::Str(""), &attributes, &[])
    };
    let declared = vec![
        declare("v_text", Some(("textcontent", "&ns_flows;")), Some("text_x5F_1")),
        declare("v_visible", Some(("visibility", "&ns_vars;")), Some("path_x5F_1")),
        declare("v_file", Some(("fileref", "&ns_vars;")), Some("image_x5F_1")),
        declare("v_graph", Some(("graphdata", "&ns_vars;")), None),
        declare("v_unbound", None, None),
    ];
    let paragraph = |text: &str| xml_element("p", &[], &[xml_text(XmlValue::Unicode(text))]);
    let row = |name: &str, text: &[&str], show: bool, file: &str, more: &[String]| {
        let mut values = vec![
            xml_element("v_text", &[], &text.iter().map(|t| paragraph(t)).collect::<Vec<_>>()),
            xml_element("v_visible", &[], &[xml_text(XmlValue::Bool(show))]),
            xml_element("v_file", &[], &[xml_text(XmlValue::Str(&format!("file:///{file}")))]),
            xml_element("v_unbound", &[], &[]),
        ];
        values.extend_from_slice(more);
        xml_node(1, "v:sampleDataSet", XmlValue::Str(""), &[("dataSetName", XmlValue::Unicode(name))], &values)
    };
    let rows = [
        row("ds_one", &["one"], true, "/Users/test/Art (one)/tile-a.eps", &[]),
        row("ds_two", &["line one", "line two & < é"], false, SAMPLE_MISSING_FILE, more),
        row("ds_three", &["three"], true, "/Users/test/Art (one)/tile-b.eps", &[]),
    ];
    let data_sets = xml_node(
        1,
        "v:sampleDataSets",
        XmlValue::Str(""),
        &[("i:activeDataSet", XmlValue::Unicode("ds_two")), ("xmlns", XmlValue::Str("&ns_custom;")), ("xmlns:v", XmlValue::Str("&ns_vars;"))],
        &rows,
    );
    let set = xml_node(
        1,
        "variableSet",
        XmlValue::Str(""),
        &[("varSetName", XmlValue::Str("binding1")), ("locked", XmlValue::Str(locked))],
        &[xml_element("variables", &[], &declared), data_sets],
    );
    xml_element("variableSets", &[("xmlns", "&ns_vars;")], &[set])
}

/// The document data with an Objects on Path record: a `/Document` dictionary whose `Constraint
/// Data` array holds a record named `name` (version 1) for the objects with the ids `ids` (the
/// path's first), with the options `options` ([`on_path_options`]).
pub fn on_path_record(name: &str, ids: &[&str], options: &str) -> String {
    let ids: String = ids.iter().map(|id| format!("%_({id}) /UnicodeString ,\n")).collect();
    format!(
        "%AI9_BeginDocumentData\n%_/Document :\n%_/Dictionary :\n%_/Array :\n%_/Dictionary :\n%_1 /Int (Constraint Verison) ,\n\
         %_({name}) /UnicodeString (Constraint Name) ,\n%_/Array :\n{ids}%_; (Constraint IDs) ,\n%_/Dictionary :\n{options}%_; (Constraint Dict) ,\n\
         %_; ,\n%_; (Constraint Data) ,\n%_; /Recorded ,\n%_;\n%AI9_EndDocumentData\n"
    )
}

/// The options of an Objects on Path record for `members` objects: Pivot `pivot` (0 to 8),
/// Rotate `rotation` (radians), spaced evenly from the start of the path, then `more` entries.
pub fn on_path_options(members: usize, pivot: u8, rotation: f64, more: &str) -> String {
    let data = "%_/Dictionary :\n%_; ,\n".repeat(members);
    let order: String = (0..members).map(|i| format!("%_{i} /Int ,\n")).collect();
    format!(
        "%_0 /Real (StartRangeFraction) ,\n%_/Array :\n{data}%_; (ConstraintArtData) ,\n%_/Array :\n{order}%_; (ObjectsOrder) ,\n\
         %_{pivot} /Int (PivotPosition) ,\n%_1 /Bool (IsEquiSpaced) ,\n%_{rotation} /Real (Rotation) ,\n{more}"
    )
}

/// The dictionary of an object of an Objects on Path group, with the id `id`: the path's
/// (`order` `None`), or the object's at `order` along the path whose frame is turned `angle`
/// radians counterclockwise.
pub fn on_path_member(id: &str, order: Option<usize>, angle: f64) -> String {
    match order {
        None => format!("%_/ArtDictionary :\n%_({id}) /String (Persistent ID) ,\n%_1 /Bool (KeyPath) ,\n%_;\n%_\n"),
        Some(i) => format!(
            "%_/ArtDictionary :\n%_{i} /Int (ConstraintArtOrderId) ,\n%_({id}) /String (Persistent ID) ,\n%_({angle}) /String (BBAccumRotation) ,\n%_;\n%_\n"
        ),
    }
}

/// A document of 200 × 100 pt with a bit of everything the reader reads. A layer `Art` that shows:
/// a group `Pair` (a red square `Red` and a hidden one), a compound path `Ring`, a clipping group
/// `Window` (a grey square seen through a smaller one), what [`page_ps`] and [`page_pdf`] draw. A
/// hidden layer `Hidden`: a radially shaded square `Shaded`, a screened square at half opacity, a
/// locked sublayer `Sub`, and a 2 × 1 image with an alpha channel.
pub fn sample_data() -> Vec<u8> {
    let red = "0 1 1 0 1 0 0 Xa\n";
    let gradient =
        "%AI5_BeginGradient: (Glow)\n(Glow) 1 2 Bd\n[\n%_0 0 0 0 1 1 1 2 1 6 50 0 Bs\n%_0 0 0 1 0 0 0 2 0.5 6 50 100 Bs\nBD\n%AI5_EndGradient\n";
    let group = format!(
        "0 Ae\nu\n{red}{}f\n{}1 Xw\n0 0 1 0 0 0 1 XA\n2 w\n{}b\n0 Xw\nU\n{}9 () XW\n",
        rect(10.0, 50.0, 40.0, 90.0),
        named("Red"),
        rect(50.0, 50.0, 80.0, 90.0),
        named("Pair")
    );
    let compound = format!("0 Ae\n*u\n{red}1 XR\n{}f\n{}f\n*U\n{}", rect(100.0, 50.0, 140.0, 90.0), rect(110.0, 60.0, 130.0, 80.0), named("Ring"));
    let clip =
        format!("0 Ae\nq\n0.5 g\n{}f\n{}h\nW\nn\nQ\n{}9 () XW\n", rect(150.0, 50.0, 190.0, 90.0), rect(160.0, 60.0, 180.0, 80.0), named("Window"));
    let shaded = format!(
        "{}Bb\n1 (Glow) 0 0 0 1 1 0 0 1 0 0 1 Bg\n20 0 0 -20 30 25 Bm\nf\n0 BB\n{}2 0.5 0 0 0 Xy\n{red}{}f\n0 1 0 0 0 Xy\n",
        rect(10.0, 5.0, 50.0, 45.0),
        named("Shaded"),
        rect(60.0, 5.0, 90.0, 45.0)
    );
    let sub = "%AI5_BeginLayer\n1 1 0 1 0 0 1 2 79 255 79 0 50 0 Lb\n(Sub) Ln\n0 g\n10 10 m\n30 30 L\nS\nLB\n%AI5_EndLayer--\n";
    let mut image = b"%AI5_File:\n%AI5_BeginRaster\n() 1 XG\n/DeviceRGB XN\n\
        [ 20 0 0 20 100 40 ] 2 1 0 Xh\n[ 20 0 0 20 100 40 ] 0 0 2 1 2 1 8 3 1 17 1 0 4 4 0 0\n%%BeginData: 10\rXI\n"
        .to_vec();
    image.extend_from_slice(&[255, 0, 0, 0, 0, 255, 255, 128]);
    image.extend_from_slice(b"%%EndData\r\nXH\r\n%AI5_EndRaster\r\nN\r\n");
    let head = format!(
        "{gradient}{}%AI5_BeginLayer\n0 1 1 1 0 0 0 1 255 79 79 0 50 0 Lb\n(Hidden) Ln\n{shaded}{sub}",
        layer("Art", &format!("{group}{compound}{clip}"))
    );
    let mut body = head.into_bytes();
    body.extend_from_slice(&image);
    body.extend_from_slice(b"LB\n%AI5_EndLayer--\n");
    let mut data = editing_data(200.0, 100.0, "").into_bytes();
    let at = data.windows(14).position(|w| w == b"%%PageTrailer\n").unwrap_or(data.len());
    data.splice(at..at, body);
    data
}

/// The sample's page in PostScript (what shows of it).
pub fn page_ps() -> &'static str {
    "1 0 0 setrgbcolor 10 50 30 40 rectfill\n\
     newpath 100 50 moveto 140 50 lineto 140 90 lineto 100 90 lineto closepath 110 60 moveto 130 60 lineto 130 80 lineto 110 80 lineto closepath eofill\n\
     0.5 setgray 160 60 20 20 rectfill\n"
}

/// The sample's page as PDF content (what shows of it).
pub fn page_pdf() -> &'static str {
    "1 0 0 rg 10 50 30 40 re f 100 50 40 40 re 110 60 20 20 re f* 0.5 g 160 60 20 20 re f"
}

/// An Illustrator EPS: its page drawing `page` (PostScript, 200 × 100 pt), and the editing data
/// `data` after it, not compressed.
pub fn eps(data: &[u8], page: &str) -> Vec<u8> {
    let mut out = format!(
        "{PS_HEADER} EPSF-3.0\n%%Creator: VectorCraft tests\n%%BoundingBox: 0 0 200 100\n%%HiResBoundingBox: 0 0 200 100\n%%EndComments\n{page}showpage\n%%EOF\n%AI9_PrivateDataBegin\n"
    )
    .into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\n%AI9_PrivateDataEnd\n");
    out
}

/// An Illustrator `.ai` file: a 200 × 100 pt PDF page drawing `page` (PDF content), carrying
/// `private` (the private data streams' contents, such as `%AI12_CompressedData` and zlib data) in
/// two streams.
pub fn ai(private: &[u8], page: &str) -> Vec<u8> {
    ai_with(private, page, "")
}

/// [`ai`] with more entries in the page dictionary (`/ArtBox [0 0 200 100]`).
pub fn ai_with(private: &[u8], page: &str, entries: &str) -> Vec<u8> {
    with_private(private, page, "AIPrivateData", entries)
}

/// A PDF saved with Illustrator's editing data: [`ai`]'s file with the two streams under
/// `AIPDFPrivateData1` and `AIPDFPrivateData2`. These key names were seen in the PDF attached to
/// #868, and nothing else of that file is used; ExifTool's PDF tag table lists them too
/// (<https://exiftool.org/TagNames/PDF.html>).
pub fn pdf(private: &[u8], page: &str) -> Vec<u8> {
    with_private(private, page, "AIPDFPrivateData", "")
}

/// The file of [`ai`] and [`pdf`], with `private` in the streams `{key}1` and `{key}2` and
/// `entries` more in the page dictionary.
fn with_private(private: &[u8], page: &str, key: &str, entries: &str) -> Vec<u8> {
    let (first, second) = private.split_at(private.len() / 2);
    let stream = |part: &[u8]| {
        let mut s = format!("<< /Length {} >>\nstream\n", part.len()).into_bytes();
        s.extend_from_slice(part);
        s.extend_from_slice(b"\nendstream");
        s
    };
    let objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] {entries} /Contents 4 0 R \
             /Resources << /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >> \
             /PieceInfo << /{PIECE} << /Private << /{key}1 5 0 R /{key}2 6 0 R /NumBlock 2 >> >> >> >>"
        )
        .into_bytes(),
        stream(page.as_bytes()),
        stream(first),
        stream(second),
    ];
    let mut out = b"%PDF-1.5\n".to_vec();
    let mut offsets = vec![];
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
    for off in offsets {
        out.extend(format!("{off:010} 00000 n \n").bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).bytes());
    out
}
