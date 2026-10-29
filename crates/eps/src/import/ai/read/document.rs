use super::art::{element_name, elements, join_and, sane};
use super::containers::drop_commented_copies;
use super::types::{FLIP, Kind, MAX_DEPTH};
use super::{Affine, Artboard, ColorMode, Document, NodeKind, Obj, OpacityMask, Paint, Point, Reader, Rect, Structure, Swatch, Unit, V, obj};
use vectorcraft_doc::{ImageBlob, PatternDef};

/// `%AI5_RulerUnits` → [`Unit`] (Illustrator 7 spec table 3; centimeters is 4; pixels is 6 in later files).
fn ruler_unit(v: u8) -> Option<Unit> {
    Some(match v {
        0 => Unit::Inches,
        1 => Unit::Millimeters,
        2 => Unit::Points,
        3 => Unit::Picas,
        4 => Unit::Centimeters,
        6 => Unit::Pixels,
        _ => return None,
    })
}

/// The name of the empty group that stands for a text object, followed by the number of its
/// story in the text document when the file says.
pub(crate) const TEXT_SLOT: &str = "\u{0}text";

/// Is `name` that of a text slot, and which story is it of?
pub(crate) fn slot_of(name: Option<&str>) -> Option<Option<u32>> {
    name?.strip_prefix(TEXT_SLOT).map(|rest| rest.split(':').next().and_then(|s| s.parse().ok()))
}

/// Which of its story's frames a text slot named `name` is (0 when it doesn't say).
pub(crate) fn slot_frame(name: Option<&str>) -> usize {
    name.and_then(|n| n.strip_prefix(TEXT_SLOT)).and_then(|rest| rest.split_once(':')).and_then(|(_, f)| f.parse().ok()).unwrap_or(0)
}

/// The artboards (names, and rects in art space) of the editing data `data` that [`read`] can't
/// read as a whole, as far as it gets: where to put the pages of its PDF part instead, and what to
/// call them (#1068).
pub fn artboards(data: &[u8]) -> Vec<(String, Rect)> {
    let found = artboards_read(data);
    if !found.is_empty() {
        return found;
    }
    // Reading can stop before the artboards (art kept in the document's data comes first): read
    // their array on its own.
    artboard_array(data).map_or(found, |array| {
        let mut alone = b"%_/Document :\n%_/Dictionary :\n".to_vec();
        alone.extend_from_slice(array);
        alone.extend_from_slice(b"\n%_; /NotRecorded ,\n%_;\n");
        artboards_read(&alone)
    })
}

/// The `ArtboardArray` entry of the editing data `data`, from its `/Array :` to its key: found
/// from the key back, counting the dictionaries its artboards open and close.
fn artboard_array(data: &[u8]) -> Option<&[u8]> {
    const KEY: &[u8] = b"%_; (ArtboardArray) ,";
    let end = data.windows(KEY.len()).rposition(|w| w == KEY)? + KEY.len();
    let head = data.get(..end)?;
    let mut depth = 0usize;
    let mut line_end = head.len();
    loop {
        let line_start = head.get(..line_end)?.iter().rposition(|&b| b == b'\n' || b == b'\r').map_or(0, |at| at + 1);
        if let Some(text) = head.get(line_start..line_end)?.strip_prefix(b"%_") {
            if text.starts_with(b";") || text.starts_with(b"/Def ;") {
                depth += 1;
            } else if text.starts_with(b"/") && text.windows(2).any(|w| w == b" :") {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return head.get(line_start..);
                }
            }
        }
        line_end = line_start.checked_sub(1)?;
    }
}

/// The palette of a `.ai` file's editing data, as [`palette`] reads it.
pub struct PaletteSwatches {
    pub swatches: Vec<Swatch>,
    pub groups: Vec<vectorcraft_color::swatch::SwatchGroup>,
    /// The patterns the editing data defines that the given patterns lack and a swatch paints with.
    pub patterns: Vec<PatternDef>,
    /// The image blobs the art of `patterns` shows, by key.
    pub images: std::collections::BTreeMap<String, ImageBlob>,
}

/// The swatches and color groups of the palette of the editing data `data`, for a file whose
/// layers aren't read: the swatches its PDF part has are the defaults (#4036). They come as
/// [`read`] makes them, with pattern swatches only for the patterns in `patterns` and those the
/// editing data defines; `None` when the palette is empty or reading stops before it.
pub fn palette(data: &[u8], patterns: &[PatternDef]) -> Option<PaletteSwatches> {
    let mut r = Reader::new(data);
    // The palette, the gradients and the patterns it names come before the art, so an error in the
    // art leaves them read.
    let _ = r.run();
    let mut known = patterns.to_vec();
    let defined = std::mem::take(&mut r.doc.patterns);
    known.extend(defined.iter().filter(|d| !patterns.iter().any(|p| p.name == d.name)).cloned());
    let (swatches, groups) = r.palette_swatches(&known)?;
    let painted = |name: &str| {
        swatches.iter().chain(groups.iter().flat_map(|g| &g.swatches)).any(|s| matches!(&s.paint, Paint::Pattern { pattern, .. } if pattern == name))
    };
    let added: Vec<PatternDef> = defined.into_iter().filter(|d| !patterns.iter().any(|p| p.name == d.name) && painted(&d.name)).collect();
    let mut shown = Document::new(1.0, 1.0);
    shown.patterns.clone_from(&added);
    let mut images = std::collections::BTreeMap::new();
    shown.visit_images(|_, im| {
        if let Some(blob) = r.doc.images.get(&im.key) {
            images.insert(im.key.clone(), blob.clone());
        }
    });
    Some(PaletteSwatches { swatches, groups, patterns: added, images })
}

/// [`artboards`] as far as reading `data` from its start gets: those read before an error, and
/// those of a `/Document` still open when it stopped (its art can be what stopped it).
fn artboards_read(data: &[u8]) -> Vec<(String, Rect)> {
    let mut r = Reader::new(data);
    let _ = r.run();
    let open: Vec<Obj> = r.frames.iter().filter_map(|f| if let Kind::Obj(o) = &f.kind { Some((**o).clone()) } else { None }).collect();
    for o in &open {
        if r.artboards.is_empty() {
            r.document(o);
        }
    }
    r.artboards
}

/// How many variables and data sets the variable library of the editing data has: XML nodes
/// (`/XMLNode`) of the `/Document` dictionary, a `variableSets` element holding `variableSet`s, each
/// with its `variables` (`variable` elements) and `sampleDataSets` (`sampleDataSet` elements).
/// VectorCraft doesn't read them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VariableLibrary {
    pub variables: usize,
    pub data_sets: usize,
}

impl VariableLibrary {
    /// The import's note that the library wasn't read, when it has a variable or a data set.
    pub fn note(self) -> Option<String> {
        let Self { variables, data_sets } = self;
        // Counts after a colon need no plural, so every language can translate the note whole.
        (variables + data_sets > 0).then(|| {
            format!("the file's variables and data sets weren't read (variables: {variables}, data sets: {data_sets}): the Variables panel doesn't list them")
        })
    }
}

/// The variable library of the editing data `data` ([`VariableLibrary`]), for a file whose layers
/// [`read`] can't read. Data without a `variableSets` element isn't read.
pub fn variables(data: &[u8]) -> VariableLibrary {
    if crate::import::ai::find(data, b"variableSets").is_none() {
        return VariableLibrary::default();
    }
    let mut r = Reader::new(data);
    let _ = r.run();
    // A `/Document` still open when reading stopped.
    let open: Vec<Obj> = r.frames.iter().filter_map(|f| if let Kind::Obj(o) = &f.kind { Some((**o).clone()) } else { None }).collect();
    for o in open.iter().filter(|o| o.ty == "Document") {
        variable_library(o, 0, &mut r.variables);
    }
    if r.variables != VariableLibrary::default() {
        return r.variables;
    }
    // Reading can stop before the library: read its node on its own.
    variable_sets_node(data).map_or(r.variables, |node| {
        let mut alone = b"%_/Document :\n%_/Dictionary :\n%_/Array :\n".to_vec();
        alone.extend_from_slice(node);
        alone.extend_from_slice(b"\n%_; (xmlnode-children) ,\n%_; /Recorded ,\n%_;\n");
        let mut r = Reader::new(&alone);
        let _ = r.run();
        r.variables
    })
}

/// The `variableSets` XML node of the editing data `data`, from its `/XMLNode :` line to the line
/// that closes it: found from its name back, counting the dictionaries and arrays it holds.
fn variable_sets_node(data: &[u8]) -> Option<&[u8]> {
    const NAME: &[u8] = b"variableSets) /String (xmlnode-nodename) ,";
    let at = data.windows(NAME.len()).position(|w| w == NAME)?;
    let opens = |text: &[u8]| text.starts_with(b"/") && text.windows(2).any(|w| w == b" :");
    let closes = |text: &[u8]| text.starts_with(b";") || text.starts_with(b"/Def ;");
    let mut depth = 0usize;
    let mut line_end = data.get(..at)?.iter().rposition(|&b| b == b'\n' || b == b'\r')?;
    let start = loop {
        let line_start = data.get(..line_end)?.iter().rposition(|&b| b == b'\n' || b == b'\r').map_or(0, |at| at + 1);
        if let Some(text) = data.get(line_start..line_end)?.strip_prefix(b"%_") {
            if closes(text) {
                depth += 1;
            } else if opens(text) {
                match depth.checked_sub(1) {
                    Some(d) => depth = d,
                    None => break line_start,
                }
            }
        }
        line_end = line_start.checked_sub(1)?;
    };
    let mut line_start = start;
    loop {
        let rest = data.get(line_start..)?;
        let line_end = line_start + rest.iter().position(|&b| b == b'\n' || b == b'\r').unwrap_or(rest.len());
        if let Some(text) = data.get(line_start..line_end)?.strip_prefix(b"%_") {
            if closes(text) {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return data.get(start..line_end);
                }
            } else if opens(text) {
                depth += 1;
            }
        }
        if line_end >= data.len() {
            return None;
        }
        line_start = line_end + 1;
    }
}

impl<'a> Reader<'a> {
    /// An object's dictionary: its name (`AIArtName`, or the XML id `AI10_ArtUID`), and the
    /// opacity mask it may carry.
    pub(super) fn art_dictionary(&mut self, o: &Obj) {
        // A blend's key object or spine.
        if let Some(kind) = o.text("ShapeBlendPathType")
            && let Some(id) = self.last_object().map(|n| n.id)
            && self.blend_roles.len() < 100_000
        {
            self.blend_roles.insert(id, (kind, o.text("ShapeBlendPathID")));
        }
        // An envelope (a distortion, `AIDeformStyle` / `DeformValue`) is written on `%_` lines as
        // a group holding the mesh that shapes it; the distorted art the file's PDF part draws
        // follows on plain lines. That mesh isn't art, so it goes (#2405).
        if (o.get("AIDeformStyle").is_some() || o.get("DeformValue").is_some()) && self.drop_envelope_mesh() {
            return;
        }
        if let Some(mask) = o.obj("AI9_Mask").or_else(|| o.obj("Mask")) {
            if let Some(art) = &mask.art {
                let on = |key: &str| mask.nums(key).first().copied() == Some(1.0);
                let value =
                    OpacityMask { art: art.clone(), clip: on("Clipping"), invert: on("Inverted"), disabled: on("Disabled"), linked: on("Linked") };
                if let Some(n) = self.last_object().filter(|n| !matches!(n.kind, NodeKind::Layer { .. })) {
                    n.extras_mut().mask = Some(Box::new(value));
                } else if !on("Disabled") {
                    self.unreadable("opacity masks without an owning object");
                }
            } else if mask.nums("Disabled").first().copied() != Some(1.0) {
                self.unreadable("opacity masks without readable art");
            }
        }
        let name = match o.text("AIArtName") {
            Some(name) => name,
            None => match o.obj("AI10_ArtUID").and_then(Obj::content_text) {
                Some(id) => obj::xml_name(&id),
                None => return,
            },
        };
        let Some(n) = self.last_object().filter(|n| !matches!(n.kind, NodeKind::Layer { .. }) && !name.is_empty()) else { return };
        if slot_of(n.name.as_deref()).is_some() {
            let id = n.id;
            self.slot_names.insert(id, name);
        } else {
            n.name = Some(name);
        }
    }

    /// The `/Document` dictionary: the artboards, and how many variables and data sets its variable
    /// library has.
    pub(super) fn document(&mut self, o: &Obj) {
        variable_library(o, 0, &mut self.variables);
        let Some(list) = find_entry(o, "ArtboardArray", 0) else { return };
        for (i, ab) in list.items().enumerate().take(1000) {
            let (p1, p2) = (ab.nums("PositionPoint1"), ab.nums("PositionPoint2"));
            let ([x0, y0, ..], [x1, y1, ..]) = (p1.as_slice(), p2.as_slice()) else { continue };
            let r = Rect::new(*x0, *y0, *x1, *y1).abs();
            if !sane(r) || r.area() <= 0.0 {
                continue;
            }
            let name = ab.text("Name").unwrap_or_else(|| format!("Artboard {}", i + 1));
            self.artboards.push((name, r));
        }
    }

    // ---- the document ----------------------------------------------------------------------

    /// The artboard of a file without artboards or crop marks: its size around the template
    /// box's centre (half a point off the grid).
    fn sized_board(&self) -> Option<Rect> {
        let ((w, h), (cx, cy)) = (self.art_size?, self.template_center?);
        // Rounded out, but not off a whole point the decimals in the file only just miss.
        let near = |v: f64| (v - v.round()).abs() < 1e-6;
        let (left, top) = (cx - w / 2.0, cy + h / 2.0);
        let x0 = if near(left) { left.round() } else { left.floor() };
        let y1 = if near(top) { top.round() } else { top.ceil() };
        let r = Rect::new(x0, y1 - h, x0 + w, y1);
        (sane(r) && w > 0.0 && h > 0.0).then_some(r)
    }

    /// The first artboard: the `/Document`'s first, else the crop marks, else [`Self::sized_board`].
    fn first_board(&self) -> Option<Rect> {
        self.artboards.first().map(|(_, r)| *r).or(self.cropmarks).or_else(|| self.sized_board())
    }

    /// The page of a file that says nothing of its artboard (the legacy format as other apps write
    /// it): its art's bounding box.
    fn bbox_board(&self) -> Option<Rect> {
        let [x0, y0, x1, y1] = self.hires.or(self.bbox)?;
        Some(Rect::new(x0, y0, x1, y1)).filter(|r| r.width() > 0.0 && r.height() > 0.0)
    }

    /// Art space → the document (fixed when the art starts, from the artboards read by then). A
    /// Large Canvas editing copy is written at 1/N scale ([`Self::large_canvas`]): the factor makes
    /// its art and boxes reach the page's size, which is what the page it is opened beside holds.
    pub(super) fn ensure_space(&mut self) -> Affine {
        if let Some(m) = self.to_doc {
            return m;
        }
        let first = self.first_board().or_else(|| self.bbox_board());
        let top_left = first.map_or(Point::new(0.0, 792.0), |r| Point::new(r.x0, r.y1));
        let s = self.large_canvas.unwrap_or(1.0);
        let m = Affine::new([s, 0.0, 0.0, -s, -s * top_left.x, s * top_left.y]);
        self.to_doc = Some(m);
        m
    }

    /// Art space → where the art being read goes: the document, or while a pattern's definition is
    /// read, the pattern's space (y down).
    pub(crate) fn space(&mut self) -> Affine {
        if self.pattern.is_some() { FLIP } else { self.ensure_space() }
    }

    /// The swatches of the palette, `[None]` first, in its order, and its color groups; a gradient
    /// swatch only when its gradient was read, and a pattern swatch only for a pattern in
    /// `patterns`. `None` when the palette is empty.
    fn palette_swatches(&mut self, patterns: &[PatternDef]) -> Option<(Vec<Swatch>, Vec<vectorcraft_color::swatch::SwatchGroup>)> {
        if self.palette.is_empty() {
            return None;
        }
        let palette = std::mem::take(&mut self.palette);
        let swatch = |e: crate::import::ai::palette::Entry| -> Option<Swatch> {
            use crate::import::ai::palette::Cell;
            let (paint, global, spot) = match e.cell {
                Cell::Color { color, global, spot } => (Paint::solid(color), global, spot),
                Cell::Gradient(g) => {
                    let mut gp = vectorcraft_color::gradient::GradientPaint::new(self.gradients.get(&g)?.clone());
                    gp.swatch = Some(e.name.clone());
                    (Paint::Gradient(Box::new(gp)), false, false)
                }
                Cell::Pattern(p) => {
                    patterns.iter().find(|d| d.name == p)?;
                    (Paint::Pattern { pattern: p, xf: Affine::IDENTITY }, false, false)
                }
            };
            Some(Swatch { name: e.name, paint, global, spot })
        };
        let mut swatches = vec![Swatch { name: "[None]".into(), paint: Paint::None, global: false, spot: false }];
        swatches.extend(palette.swatches.into_iter().filter_map(swatch));
        let groups: Vec<_> = palette
            .groups
            .into_iter()
            .map(|(name, s)| vectorcraft_color::swatch::SwatchGroup { name, swatches: s.into_iter().filter_map(swatch).collect() })
            .collect();
        Some((swatches, groups))
    }

    pub(super) fn finish(mut self) -> Result<Structure, String> {
        if !self.unknown.is_empty() {
            let ops: Vec<String> = self.unknown.iter().map(|o| format!("`{o}`")).collect();
            self.unsupported.insert(format!("operators VectorCraft doesn't know ({})", ops.join(", ")));
        }
        if !self.unsupported.is_empty() {
            let list: Vec<&str> = self.unsupported.iter().map(String::as_str).collect();
            return Err(format!("it has {}, which VectorCraft doesn't read from it yet", join_and(&list)));
        }
        if self.doc.layers.is_empty() {
            return Err("it has no layers".into());
        }
        let to_doc = self.ensure_space();
        let artboard = self.first_board();
        drop_commented_copies(&mut self.doc.layers, &self.hidden_ids);
        let mut doc = std::mem::replace(&mut self.doc, Document::new(1.0, 1.0));
        let boards: Vec<(String, Rect)> = if self.artboards.is_empty() {
            artboard.or_else(|| self.bbox_board()).map(|r| vec![("Artboard 1".to_string(), r)]).unwrap_or_default()
        } else {
            self.artboards.clone()
        };
        doc.artboards = boards
            .into_iter()
            .enumerate()
            .map(|(i, (name, r))| Artboard {
                id: i as u32 + 1,
                name,
                rect: to_doc.transform_rect_bbox(r),
                locked: false,
                show_center_mark: false,
                show_cross_hairs: false,
                show_video_safe_areas: false,
                video_ruler_pixel_aspect_ratio: 1.0,
                color: None,
            })
            .collect();
        if doc.artboards.is_empty() {
            let rect = doc.art_bounds().filter(|r| sane(*r) && r.area() > 0.0).unwrap_or(Rect::new(0.0, 0.0, 612.0, 792.0));
            doc.artboards = vec![Artboard {
                id: 1,
                name: "Artboard 1".into(),
                rect,
                locked: false,
                show_center_mark: false,
                show_cross_hairs: false,
                show_video_safe_areas: false,
                video_ruler_pixel_aspect_ratio: 1.0,
                color: None,
            }];
        }
        if self.color_model.unwrap_or(self.cmyk_colors > self.rgb_colors) {
            doc.color_mode = ColorMode::Cmyk;
            let (swatches, groups) = vectorcraft_color::default_swatches(ColorMode::Cmyk.model());
            (doc.swatches, doc.swatch_groups) = (swatches.into(), groups.into());
        }
        if let Some(u) = self.ruler_units.and_then(ruler_unit) {
            doc.units = u;
        }
        // The file's own swatches in place of the defaults, in its order and colour groups.
        if let Some((swatches, groups)) = self.palette_swatches(&doc.patterns) {
            (doc.swatches, doc.swatch_groups) = (swatches.into(), groups.into());
        }
        // The colour operators' definitions first: a gradient, defined before the art, names a
        // global process colour as a stop and would otherwise make it an ink (#1771).
        for n in self.spots.iter().chain(&self.stop_spots) {
            if doc.swatch(&n.name).is_none() {
                doc.swatches.push(Swatch { name: n.name.clone(), paint: Paint::solid(n.color), global: true, spot: n.spot });
            }
        }
        for name in doc.patterns.iter().map(|p| p.name.clone()).collect::<Vec<_>>() {
            if doc.swatch(&name).is_none() {
                doc.swatches.push(Swatch { paint: Paint::Pattern { pattern: name.clone(), xf: Affine::IDENTITY }, name, global: false, spot: false });
            }
        }
        let mut warnings = self.warnings;
        if self.drawn_looks > 0 {
            warnings.push(format!(
                "objects with several fills or strokes, effects or brushes came in as their drawn look ({}): their appearance isn't editable yet",
                self.drawn_looks
            ));
        }
        warnings.extend(self.variables.note());
        Ok(Structure {
            doc,
            to_doc,
            bbox: self.hires.or(self.bbox),
            artboard,
            template: self.template,
            hidden_unread: self.hidden_unread,
            slot_names: self.slot_names,
            restyled: self.restyled,
            warnings,
        })
    }
}

/// The dictionary of entry `key` in `o` or the dictionaries it holds.
fn find_entry<'o>(o: &'o Obj, key: &str, depth: usize) -> Option<&'o Obj> {
    if depth > 8 {
        return None;
    }
    o.obj(key).or_else(|| o.entries.iter().flat_map(|(_, v)| v.iter()).filter_map(V::obj).find_map(|c| find_entry(c, key, depth + 1)))
}

/// Add the variables and data sets of the variable libraries in `o` or the dictionaries it holds
/// (see [`VariableLibrary`]) to `found`.
fn variable_library(o: &Obj, depth: usize, found: &mut VariableLibrary) {
    if depth > MAX_DEPTH {
        return;
    }
    if o.ty == "XMLNode" && element_name(o).as_deref() == Some("variableSets") {
        for set in elements(o, "variableSet") {
            found.variables += elements(set, "variables").flat_map(|v| elements(v, "variable")).count();
            found.data_sets += elements(set, "sampleDataSets").flat_map(|d| elements(d, "sampleDataSet")).count();
        }
        return;
    }
    for c in o.items() {
        variable_library(c, depth + 1, found);
    }
}
