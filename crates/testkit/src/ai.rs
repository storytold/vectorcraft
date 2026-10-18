//! Hand-written Illustrator editing data (the format `vectorcraft_eps`'s reader documents) and the
//! EPS, `.ai` and PDF files that carry it, for import tests. Nothing here comes from files another
//! app wrote, except the key names that [`pdf`] uses and the XML node keys of [`variable_library`].

/// Editing data of a `width` × `height` page: header comments (the art's bounding box and the crop
/// marks are the page), `body` (setup and layers), the trailer.
pub fn editing_data(width: f64, height: f64, body: &str) -> String {
    format!(
        "{PS_HEADER}\n%%Creator: VectorCraft tests\n%%BoundingBox: 0 0 {width} {height}\n%%HiResBoundingBox: 0 0 {width} {height}\n\
         %AI5_FileFormat 14.0\n%AI3_Cropmarks: 0 0 {width} {height}\n%%EndComments\n\
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

/// The document data with a variable library: a `/Document` dictionary whose `Recorded` dictionary
/// holds the library's XML document as XML nodes (`/XMLNode`) under `#document`. The library has
/// one variable set, the text variables `variables` and the data sets `data_sets`, each with a `p`
/// element for every variable.
pub fn variable_library(variables: &[&str], data_sets: &[&str]) -> String {
    // A node: its name, value and type (1 an element, 2 an attribute, 3 text, 9 the document), its
    // children and its attributes.
    fn node(kind: u8, name: &str, value: &str, attributes: &[(&str, &str)], children: &[String]) -> String {
        let attributes: String = attributes.iter().map(|(k, v)| node(2, k, v, &[], &[node(3, "#text", v, &[], &[])])).collect();
        format!(
            "%_/XMLNode :\n%_({name}) /String (xmlnode-nodename) ,\n%_({value}) /String (xmlnode-nodevalue) ,\n%_{kind} /Int (xmlnode-nodetype) ,\n\
             %_/Array :\n{}%_; (xmlnode-children) ,\n%_/Array :\n{attributes}%_; (xmlnode-attributes) ,\n%_;\n",
            children.concat()
        )
    }
    let element = |name: &str, attributes: &[(&str, &str)], children: &[String]| node(1, name, "", attributes, children);
    let declared: Vec<String> = variables.iter().map(|v| element("variable", &[("varName", v), ("trait", "textcontent")], &[])).collect();
    let sets: Vec<String> = data_sets
        .iter()
        .map(|d| {
            let values: Vec<String> =
                variables.iter().map(|v| element(v, &[], &[element("p", &[], &[node(3, "#text", &format!("{d} {v}"), &[], &[])])])).collect();
            element("v:sampleDataSet", &[("dataSetName", d)], &values)
        })
        .collect();
    let set = element(
        "variableSet",
        &[("varSetName", "binding1"), ("locked", "none")],
        &[element("variables", &[], &declared), element("v:sampleDataSets", &[("xmlns", "&ns_custom;"), ("xmlns:v", "&ns_vars;")], &sets)],
    );
    let library = element("metadata", &[], &[element("variableSets", &[("xmlns", "&ns_vars;")], &[set])]);
    format!(
        "%AI9_BeginDocumentData\n%_/Document :\n%_/Dictionary :\n{}%_; (#document) ,\n%_; /Recorded ,\n%_;\n%AI9_EndDocumentData\n",
        node(9, "#document", "", &[], &[library])
    )
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
