//! Every global and member the script host (`script.run`) adds to the JavaScript engine is in one of
//! the tables below, and each name that reaches outside the document has the `outside` id
//! `script.check` (`vectorcraft-scriptcheck`) gives it. The names are read from a run: the own
//! properties of the global object, of each object or function it holds and of each function's
//! `prototype`, with a setter listed again as `name=`. The names a plain `vectorcraft_js::Realm` also
//! has are the language's, which reach files and other apps only through the host; `eval` and
//! `Function` still have rows, since they run code the checker can't read. A name the host adds
//! without a row fails the test.

use std::collections::BTreeSet;

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_js::{DirectiveName, JsValue, Realm};
use vectorcraft_scriptcheck::{Limits, check};

/// The names that reach outside the document: each with a script that uses it and the `outside` id
/// the checker gives that script.
const OUTSIDE: &[(&str, &str, &str)] = &[
    ("eval", "eval(s);", "unseenCode"),
    ("Function", "var g = new Function(s);", "unseenCode"),
    ("$.evalFile", "$.evalFile(p);", "unseenCode"),
    ("$.getenv", "var h = $.getenv(n);", "environment"),
    ("$.setenv", "$.setenv(n, v);", "environment"),
    ("File.prototype.open", "var f = new File(p); f.open('w');", "filesWritten"),
    ("File.prototype.write", "var f = new File(p); f.write(s);", "filesWritten"),
    ("File.prototype.writeln", "var f = new File(p); f.writeln(s);", "filesWritten"),
    ("File.prototype.copy", "var f = new File(p); f.copy(q);", "filesWritten"),
    ("File.prototype.createAlias", "var f = new File(p); f.createAlias(q);", "filesWritten"),
    ("File.prototype.remove", "var f = new File(p); f.remove();", "filesWritten"),
    ("File.prototype.rename", "var f = new File(p); f.rename(q);", "filesWritten"),
    ("File.prototype.length=", "var f = new File(p); f.length = 0;", "filesWritten"),
    ("File.prototype.hidden=", "var f = new File(p); f.hidden = true;", "filesWritten"),
    ("File.prototype.readonly=", "var f = new File(p); f.readonly = true;", "filesWritten"),
    ("File.prototype.execute", "var f = new File(p); f.execute();", "filesOpened"),
    ("Folder.prototype.create", "var d = new Folder(p); d.create();", "filesWritten"),
    ("Folder.prototype.remove", "var d = new Folder(p); d.remove();", "filesWritten"),
    ("Folder.prototype.rename", "var d = new Folder(p); d.rename(q);", "filesWritten"),
    ("Folder.prototype.execute", "var d = new Folder(p); d.execute();", "filesOpened"),
];

/// The names the host adds that stay inside the document and the run, by the object that has them.
/// Reading a file or a folder, a file dialog and a message to the person do not change anything
/// outside it. The document model (`app`, its classes and the enumerations) acts on the open
/// documents; `Document.saveAs` and `close` throw "not available yet".
const INSIDE: &[(&str, &str)] = &[
    ("", "$ File Folder UnitValue alert confirm localize prompt toSource"),
    ("Object.prototype.", "toSource"),
    ("UnitValue.", "baseUnit baseUnit= length prototype"),
    ("UnitValue.prototype.", "as baseUnit baseUnit= constructor convert toString type type= value value="),
    ("localize.", "length"),
    ("toSource.", "length"),
    (
        "$.",
        "about appEncoding appEncoding= bp build buildDate colorPicker decimalPoint engineName error fileName flags \
         flags= gc global hiresTimer includePath level line locale locale= localize localize= memCache memCache= os screens \
         sleep stack strict version write writeln",
    ),
    ("File.", "decode encode fs isEncodingAvailable openDialog saveDialog length prototype"),
    (
        "File.prototype.",
        "absoluteURI alias changePath close constructor created creator displayName encoding encoding= eof error error= \
         exists fsName fullName getRelativeURI hidden length lineFeed lineFeed= localizedName modified name openDlg parent path read \
         readch readln readonly relativeURI resolve saveDlg seek tell toString type",
    ),
    (
        "Folder.",
        "appData appPackage commonFiles current current= decode desktop encode fs isEncodingAvailable length myDocuments \
         prototype selectDialog startup system temp trash userData",
    ),
    (
        "Folder.prototype.",
        "absoluteURI alias changePath constructor created displayName error error= exists fsName fullName getFiles \
         getRelativeURI localizedName modified name parent path relativeURI resolve selectDlg toString",
    ),
    (
        "",
        "activeDocument activeDocument= app Application CoordinateSystem Document DocumentArtboardLayout DocumentColorSpace \
         DocumentPreset DocumentPresetType DocumentPreviewMode DocumentRasterResolution DocumentTransparencyGrid documents \
         Documents ElementPlacement Layer PageItem PageItems RulerUnits selection selection= ZOrderMethod",
    ),
    ("Application.", "prototype"),
    (
        "Application.prototype.",
        "activeDocument activeDocument= beep beep= constructor coordinateSystem coordinateSystem= documents getIsFileOpen \
         selection selection= textFonts toString typename",
    ),
    ("DocumentPreset.", "prototype"),
    (
        "DocumentPreset.prototype.",
        "artboardLayout artboardLayout= artboardRowsOrCols artboardRowsOrCols= artboardSpacing artboardSpacing= colorMode \
         colorMode= constructor documentBleedLink documentBleedLink= documentBleedOffsetRect documentBleedOffsetRect= height \
         height= numArtboards numArtboards= previewMode previewMode= rasterResolution rasterResolution= title title= \
         toString transparencyGrid transparencyGrid= typename units units= width width=",
    ),
    ("Documents.", "prototype"),
    ("Documents.prototype.", "add addDocument constructor getByName index length parent toString typename"),
    ("Document.", "prototype"),
    (
        "Document.prototype.",
        "activeLayer activeLayer= artboards artboards= characterStyles close close= constructor convertCoordinate \
         documentColorSpace groupItems groupItems= height layers layers= name pageItems paragraphStyles parent pathItems \
         pathItems= rulerOrigin rulerOrigin= rulerUnits saveAs saveAs= selection selection= swatches swatches= symbols \
         symbols= textFrames toString typename views views= width",
    ),
    ("Layer.", "prototype"),
    ("Layer.prototype.", "constructor name toString typename"),
    ("PageItems.", "prototype"),
    ("PageItems.prototype.", "constructor getByName index length parent removeAll toString typename"),
    ("PageItem.", "prototype"),
    (
        "PageItem.prototype.",
        "artworkKnockout artworkKnockout= blendingMode blendingMode= bringInPerspective bringInPerspective= constructor \
         controlBounds duplicate editable geometricBounds height height= hidden hidden= isIsolated isIsolated= layer layer= \
         left left= locked locked= move name name= note note= opacity opacity= parent pixelAligned pixelAligned= position \
         position= remove resize resize= rotate rotate= selected selected= sliced sliced= tags tags= toString top top= transform \
         transform= translate translate= typename uRL uRL= uuid uuid= visibilityVariable visibilityVariable= visibleBounds \
         width width= wrapInside wrapInside= wrapOffset wrapOffset= wrapped wrapped= zOrder zOrderPosition",
    ),
    ("CoordinateSystem.", "ARTBOARDCOORDINATESYSTEM DOCUMENTCOORDINATESYSTEM"),
    ("DocumentArtboardLayout.", "Column GridByCol GridByRow RLGridByCol RLGridByRow RLRow Row"),
    ("DocumentColorSpace.", "CMYK RGB"),
    ("DocumentPresetType.", "BasicCMYK BasicRGB Mobile Print Video Web"),
    ("DocumentPreviewMode.", "DefaultPreview OverprintPreview PixelPreview"),
    ("DocumentRasterResolution.", "HighResolution MediumResolution ScreenResolution"),
    (
        "DocumentTransparencyGrid.",
        "TransparencyGridBlue TransparencyGridDark TransparencyGridGreen TransparencyGridLight TransparencyGridMedium \
         TransparencyGridNone TransparencyGridOrange TransparencyGridPurple TransparencyGridRed",
    ),
    ("ElementPlacement.", "INSIDE PLACEAFTER PLACEATBEGINNING PLACEATEND PLACEBEFORE"),
    ("RulerUnits.", "Centimeters Inches Millimeters Picas Pixels Points Qs Unknown"),
    ("ZOrderMethod.", "BRINGFORWARD BRINGTOFRONT SENDBACKWARD SENDTOBACK"),
    (
        "",
        "CharacterAttributes CharacterStyle CharacterStyleAttributes CharacterStyles Characters Justification \
         ParagraphAttributes ParagraphStyle ParagraphStyleAttributes ParagraphStyles Paragraphs TextFont TextFonts TextFrame \
         TextFrames TextOrientation TextRange TextType Words",
    ),
    ("TextFrames.", "prototype"),
    ("TextFrames.prototype.", "add areaText constructor getByName index length parent pathText pointText removeAll typename"),
    ("TextFrame.", "prototype"),
    (
        "TextFrame.prototype.",
        "anchor anchor= antialias antialias= characters columnCount columnCount= columnGutter columnGutter= constructor \
         contents contents= convertAreaObjectToPointObject convertPointObjectToAreaObject createOutline endTValue endTValue= \
         insertionPoints insertionPoints= kind lines lines= matrix matrix= nextFrame nextFrame= opticalAlignment \
         opticalAlignment= orientation orientation= paragraphs previousFrame previousFrame= rowCount rowCount= rowGutter \
         rowGutter= startTValue startTValue= story story= textPath textPath= textRange textRanges textRanges= textSelection \
         textSelection= words",
    ),
    ("TextRange.", "prototype"),
    (
        "TextRange.prototype.",
        "autoLeading autoLeading= baselineShift baselineShift= changeCaseTo changeCaseTo= characterAttributes characterOffset \
         characterOffset= characterStyles characters constructor contents contents= deSelect deSelect= \
         duplicate duplicate= end end= getLocalCharOverridesJSON getLocalCharOverridesJSON= getLocalParaOverridesJSON \
         getLocalParaOverridesJSON= getParagraphLength getTextRunLength horizontalScale horizontalScale= insertionPoints \
         insertionPoints= kerning kerning= leading leading= length length= lines lines= move move= paragraphAttributes \
         paragraphStyles paragraphs parent remove rotation rotation= select select= size size= start start= \
         story story= strikeThrough strikeThrough= textFont textFont= textRanges textRanges= textSelection textSelection= \
         tracking tracking= typename underline underline= verticalScale verticalScale= words",
    ),
    ("Characters.", "prototype"),
    ("Characters.prototype.", "constructor length parent typename"),
    ("Words.", "prototype"),
    ("Words.prototype.", "constructor length parent typename"),
    ("Paragraphs.", "prototype"),
    ("Paragraphs.prototype.", "constructor length parent typename"),
    ("CharacterAttributes.", "prototype"),
    (
        "CharacterAttributes.prototype.",
        "autoLeading autoLeading= baselineShift baselineShift= constructor horizontalScale horizontalScale= leading leading= \
         rotation rotation= size size= strikeThrough strikeThrough= textFont textFont= tracking tracking= typename underline \
         underline= verticalScale verticalScale=",
    ),
    ("ParagraphAttributes.", "prototype"),
    (
        "ParagraphAttributes.prototype.",
        "constructor firstLineIndent firstLineIndent= justification justification= leftIndent leftIndent= rightIndent \
         rightIndent= spaceAfter spaceAfter= spaceBefore spaceBefore= typename",
    ),
    ("CharacterStyles.", "prototype"),
    ("CharacterStyles.prototype.", "add constructor getByName index length parent removeAll typename"),
    ("CharacterStyle.", "prototype"),
    ("CharacterStyle.prototype.", "applyTo characterAttributes constructor name name= parent remove typename"),
    ("CharacterStyleAttributes.", "prototype"),
    (
        "CharacterStyleAttributes.prototype.",
        "autoLeading autoLeading= baselineShift baselineShift= constructor horizontalScale horizontalScale= leading leading= \
         rotation rotation= size size= strikeThrough strikeThrough= textFont textFont= tracking tracking= typename underline \
         underline= verticalScale verticalScale=",
    ),
    ("ParagraphStyles.", "prototype"),
    ("ParagraphStyles.prototype.", "add constructor getByName index length parent removeAll typename"),
    ("ParagraphStyle.", "prototype"),
    (
        "ParagraphStyle.prototype.",
        "applyTo characterAttributes characterAttributes= constructor name name= paragraphAttributes parent remove typename",
    ),
    ("ParagraphStyleAttributes.", "prototype"),
    (
        "ParagraphStyleAttributes.prototype.",
        "constructor firstLineIndent firstLineIndent= justification justification= leftIndent leftIndent= rightIndent \
         rightIndent= spaceAfter spaceAfter= spaceBefore spaceBefore= typename",
    ),
    ("TextFonts.", "prototype"),
    ("TextFonts.prototype.", "constructor getByName index length parent typename"),
    ("TextFont.", "prototype"),
    ("TextFont.prototype.", "constructor family name parent style typename"),
    ("Justification.", "CENTER FULLJUSTIFY FULLJUSTIFYLASTLINECENTER FULLJUSTIFYLASTLINELEFT FULLJUSTIFYLASTLINERIGHT LEFT RIGHT"),
    ("TextOrientation.", "HORIZONTAL VERTICAL"),
    ("TextType.", "AREATEXT PATHTEXT POINTTEXT"),
];

/// The classes the host adds with nothing but a prototype and its constructor: the page item types.
const BARE_CLASSES: &str = "BlendItem CompoundPathItem EnvelopeItem GraphItem GroupItem IntertwineItem MeshItem PathItem \
     PlacedDocumentItem PlacedItem RasterItem RepeatItem SymbolItem";

/// The name the checker does not read, which also stands for its members: `vc` is VectorCraft's
/// own API, and what `vc.run` does depends on the command it runs.
const NOT_CHECKED: &str = "vc";

/// An expression whose value is the list of names, one to a line.
const LIST: &str = r#"(function () {
    var out = [];
    function add(o, prefix) {
        var own = Object.getOwnPropertyNames(o);
        for (var i = 0; i < own.length; i++) {
            out.push(prefix + own[i]);
            var d = Object.getOwnPropertyDescriptor(o, own[i]);
            if (d && d.set) out.push(prefix + own[i] + "=");
        }
    }
    var g = (function () { return this; })();
    var globals = Object.getOwnPropertyNames(g);
    add(g, "");
    for (var i = 0; i < globals.length; i++) {
        // A global with a getter, such as `activeDocument`, is not read: it throws with no document open.
        var gd = Object.getOwnPropertyDescriptor(g, globals[i]);
        if (gd && gd.get) continue;
        var v = g[globals[i]];
        if (v !== null && (typeof v == "object" || typeof v == "function")) {
            add(v, globals[i] + ".");
            if (typeof v == "function" && v.prototype !== null && typeof v.prototype == "object") add(v.prototype, globals[i] + ".prototype.");
        }
    }
    return out.join("\n");
})()"#;

/// The names a script run by the host has.
fn host_names() -> BTreeSet<String> {
    let mut s = Session::new();
    let r = s.execute("script.run", &json!({"source": format!("vc.log({LIST});"), "name": "list.js"})).unwrap();
    let text = r["log"][0].as_str().unwrap_or_else(|| panic!("no list in {r}"));
    text.lines().map(str::to_string).collect()
}

/// The names a plain JavaScript engine has.
fn language_names() -> BTreeSet<String> {
    match Realm::new().run(LIST) {
        Ok(JsValue::String(text)) => text.lines().map(str::to_string).collect(),
        other => panic!("no list: {other:?}"),
    }
}

fn table_names() -> BTreeSet<String> {
    let inside = INSIDE.iter().flat_map(|(prefix, names)| names.split_whitespace().map(move |n| format!("{prefix}{n}")));
    let bare = BARE_CLASSES.split_whitespace().flat_map(|c| [c.to_string(), format!("{c}.prototype"), format!("{c}.prototype.constructor")]);
    OUTSIDE.iter().map(|(n, ..)| n.to_string()).chain(inside).chain(bare).chain([NOT_CHECKED.to_string()]).collect()
}

#[test]
fn every_name_the_host_adds_has_a_row() {
    let host = host_names();
    let language = language_names();
    let table = table_names();
    let not_checked = |n: &&String| n.starts_with(&format!("{NOT_CHECKED}."));
    let missing: Vec<_> = host.difference(&language).filter(|n| !table.contains(*n) && !not_checked(n)).collect();
    assert!(missing.is_empty(), "the host adds these names, which no table here lists: {missing:?}");
    let gone: Vec<_> = table.difference(&host).collect();
    assert!(gone.is_empty(), "the tables list these names, which a script run by the host does not have: {gone:?}");
}

#[test]
fn every_name_that_reaches_outside_has_its_checker_id() {
    for (name, script, id) in OUTSIDE {
        let r = check("probe.jsx", script.as_bytes(), None, &Limits::default());
        let ids: Vec<_> = r.outside.iter().map(|o| o.id).collect();
        assert!(ids.contains(id), "{name}: the checker gives {ids:?} for {script:?}, not {id}");
    }
}

/// A line with the directive `d`. The match has no catch-all, so a new directive needs a row, and
/// [`DirectiveName::from_name`] reads only the directives in [`DirectiveName::ALL`].
fn directive_line(d: DirectiveName) -> &'static str {
    match d {
        DirectiveName::Include => "#include \"lib.jsxinc\"",
        DirectiveName::IncludePath => "#includepath \"lib\"",
        DirectiveName::Script => "#script Tidy",
        DirectiveName::Strict => "#strict on",
        DirectiveName::Target => "#target vectorcraft",
        DirectiveName::TargetEngine => "#targetengine session",
    }
}

#[test]
fn the_checker_lists_every_directive_the_host_reads() {
    for d in DirectiveName::ALL {
        let line = directive_line(d);
        let r = check("probe.jsx", format!("{line}\nvar a = 1;\n").as_bytes(), None, &Limits::default());
        let listed: Vec<_> = r.directives.iter().map(|d| d.name.to_string()).collect();
        assert_eq!(listed, [d.as_str()], "{line}");
    }
}
