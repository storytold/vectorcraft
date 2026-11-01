use super::art::{element_name, join_and, sane};
use super::containers::drop_commented_copies;
use super::types::{FLIP, Kind, MAX_DEPTH};
use super::{
    Affine, Arc, Artboard, CANVAS_CENTRE, ColorMode, Document, Node, NodeId, NodeKind, Obj, OpacityMask, Paint, Point, Reader, Rect, Structure,
    Swatch, Unit, V, obj,
};
use vectorcraft_doc::{DataSet, DataValue, Guide, ImageBlob, PatternDef, Variable, VariableKind, Variables};

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

/// The variable library of the editing data, as [`variable_library`] reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VariableRead {
    /// Its variables, data sets, active data set and lock, without bindings.
    pub variables: Variables,
    /// How many of its declarations, data sets, values and settings were left out.
    pub skipped: usize,
    /// Whether the editing data has a variable library (a `variableSets` element).
    pub present: bool,
}

impl VariableRead {
    /// The import's note about what of the library wasn't read: nothing when all of it was.
    pub fn note(&self) -> Option<String> {
        if !self.present || self.skipped == 0 {
            return None;
        }
        if self.variables.variables.is_empty() && self.variables.datasets.is_empty() {
            return Some("the file's variable library couldn't be read: the Variables panel doesn't list it".into());
        }
        // A count after a colon needs no plural, so every language can translate the note whole.
        Some(format!("the file's variable library was partly read (entries skipped: {}): the Variables panel lists the rest", self.skipped))
    }
}

/// The variable library of the editing data `data` ([`VariableRead`]), for a file whose layers
/// [`read`] can't read. Data without a `variableSets` element isn't read.
pub fn variables(data: &[u8]) -> VariableRead {
    if crate::import::ai::find(data, b"variableSets").is_none() {
        return VariableRead::default();
    }
    let mut r = Reader::new(data);
    let _ = r.run();
    if r.variables.present {
        return r.variables;
    }
    // A `/Document` still open when reading stopped.
    let open: Vec<Obj> = r.frames.iter().filter_map(|f| if let Kind::Obj(o) = &f.kind { Some((**o).clone()) } else { None }).collect();
    if let Some(found) = open.iter().filter(|o| o.ty == "Document").map(variable_library).find(|v| v.present) {
        return found;
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
        self.constraint_art(o);
        // A blend's key object or spine.
        if let Some(kind) = o.text("ShapeBlendPathType")
            && let Some(id) = self.last_object().map(|n| n.id)
            && self.blend_roles.len() < 100_000
        {
            self.blend_roles.insert(id, (kind, o.text("ShapeBlendPathID")));
        }
        // An envelope (a distortion, `AIDeformStyle` / `DeformValue`) is written on `%_` lines as
        // a group holding the mesh that shapes it (or its contents, while they are being edited);
        // the distorted art the file's PDF part draws follows on plain lines. That group isn't
        // art, so it goes (#2405), kept aside for the live envelope ([`Reader::end_envelope`]).
        if (o.get("AIDeformStyle").is_some() || o.get("DeformValue").is_some()) && self.drop_envelope_art() {
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
                    self.unreadable("opacity masks without an owning object", false);
                }
                // What the art doesn't read matters only when the mask shapes what shows.
                for what in &mask.unread {
                    if on("Disabled") {
                        self.hidden_unread.insert(what.clone());
                    } else if let Some(op) = what.strip_prefix('`').and_then(|w| w.strip_suffix('`')) {
                        self.note_unknown(op, false);
                    } else {
                        self.unreadable(what, false);
                    }
                }
            } else if mask.nums("Disabled").first().copied() != Some(1.0) {
                self.unreadable("opacity masks without readable art", false);
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

    /// The `/Document` dictionary: the artboards, its variable library (the first one read), and its
    /// Objects on Path records.
    pub(super) fn document(&mut self, o: &Obj) {
        if !self.variables.present {
            self.variables = variable_library(o);
        }
        self.constraint_data(o);
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

    /// The page the art is compared with in the document's space, once the art has started.
    pub(super) fn page_in_doc(&self) -> Option<Rect> {
        let board = self.first_board().or_else(|| self.bbox_board())?;
        Some(self.to_doc?.transform_rect_bbox(board))
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
        // Envelopes whose plugin never closed them keep their drawn art.
        if !self.envelopes.is_empty() {
            self.envelopes.clear();
            self.warn(crate::import::ai::envelope::DRAWN);
        }
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
        super::on_path::apply(&mut self.doc, &self.on_path);
        let mut doc = std::mem::replace(&mut self.doc, Document::new(1.0, 1.0));
        let boards: Vec<(String, Rect)> = if self.artboards.is_empty() {
            artboard.or_else(|| self.bbox_board()).map(|r| vec![("Artboard 1".to_string(), r)]).unwrap_or_default()
        } else {
            self.artboards.clone()
        };
        let board_rects: Vec<Rect> = boards.iter().map(|(_, r)| *r).collect();
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
        if !self.ruler_guides.is_empty() {
            let ids: Vec<u32> = doc.artboards.iter().map(|a| a.id).collect();
            let (candidates, centre) = (std::mem::take(&mut self.ruler_guides), self.template_center);
            let pick = |id: NodeId| -> Option<Guide> {
                let (a, b) = candidates.get(&id)?;
                let (vertical, board) = ruler_guide(*a, *b, centre, &board_rects)?;
                let p = to_doc * *a;
                let artboard = match board {
                    Some(i) => Some(*ids.get(i)?),
                    None => None,
                };
                let pos = if vertical { p.x } else { p.y };
                pos.is_finite().then_some(Guide { vertical, pos, artboard, layer: None })
            };
            take_ruler_guides(&mut doc.layers, &pick, &mut doc.guides, 0);
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
        let variables = std::mem::take(&mut self.variables);
        warnings.extend(variables.note());
        if variables.present {
            doc.variables = variables.variables.into();
        }
        Ok(Structure {
            doc,
            to_doc,
            bbox: self.hires.or(self.bbox),
            artboard,
            template: self.template,
            hidden_unread: self.hidden_unread,
            unread: self.unread,
            unread_elsewhere: self.unread_elsewhere,
            slot_names: self.slot_names,
            restyled: self.restyled,
            placed_elsewhere: self.placed_elsewhere,
            envelopes_drawn: self.envelopes_drawn,
            warnings,
        })
    }
}

/// The dictionary of entry `key` in `o` or the dictionaries it holds.
/// How far (points) a ruler guide's ends may lie from the canvas's or an artboard's edges.
const GUIDE_TOLERANCE: f64 = 0.01;

/// Whether the straight guide path from `a` to `b` (art space) is a ruler guide → whether it is
/// vertical, and the index in `boards` of the artboard it belongs to (`None`: a canvas guide). A
/// canvas guide runs across the whole canvas, 16,383 points centred on the template box's
/// centre `centre`; an artboard guide runs from edge to edge of one artboard of `boards`, inside it.
fn ruler_guide(a: Point, b: Point, centre: Option<(f64, f64)>, boards: &[Rect]) -> Option<(bool, Option<usize>)> {
    let near = |u: f64, v: f64| (u - v).abs() <= GUIDE_TOLERANCE;
    let vertical = match (near(a.x, b.x), near(a.y, b.y)) {
        (true, false) => true,
        (false, true) => false,
        _ => return None,
    };
    let (along, across) = if vertical { ((a.y.min(b.y), a.y.max(b.y)), a.x) } else { ((a.x.min(b.x), a.x.max(b.x)), a.y) };
    if let Some((cx, cy)) = centre {
        let c = if vertical { cy } else { cx };
        if near(along.0, c - CANVAS_CENTRE) && near(along.1, c + CANVAS_CENTRE) {
            return Some((vertical, None));
        }
    }
    let mut on = boards.iter().enumerate().filter(|(_, r)| {
        let (ends, sides) =
            if vertical { ((r.min_y(), r.max_y()), (r.min_x(), r.max_x())) } else { ((r.min_x(), r.max_x()), (r.min_y(), r.max_y())) };
        near(along.0, ends.0) && near(along.1, ends.1) && (sides.0..=sides.1).contains(&across)
    });
    match (on.next(), on.next()) {
        (Some((i, _)), None) => Some((vertical, Some(i))),
        _ => None,
    }
}

/// Moves the children of the layers and sublayers of `nodes` that `pick` makes a guide of into
/// `guides`, each on its layer.
fn take_ruler_guides(nodes: &mut [Arc<Node>], pick: &dyn Fn(NodeId) -> Option<Guide>, guides: &mut Vec<Guide>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for n in nodes {
        if !matches!(n.kind, NodeKind::Layer { .. }) {
            continue;
        }
        let id = n.id;
        let Some(kids) = Arc::make_mut(n).children_mut() else { continue };
        kids.retain(|k| match pick(k.id) {
            Some(g) => {
                guides.push(Guide { layer: Some(id), ..g });
                false
            }
            None => true,
        });
        take_ruler_guides(kids, pick, guides, depth + 1);
    }
}

pub(super) fn find_entry<'o>(o: &'o Obj, key: &str, depth: usize) -> Option<&'o Obj> {
    if depth > 8 {
        return None;
    }
    o.obj(key).or_else(|| o.entries.iter().flat_map(|(_, v)| v.iter()).filter_map(V::obj).find_map(|c| find_entry(c, key, depth + 1)))
}

// ---- the variable library -----------------------------------------------------------------

/// Caps on the variable library of the editing data, as Load Variable Library has them: the XML
/// nodes read, the variables and data sets kept, the characters of a name, and the bytes of the
/// names and values kept.
const MAX_LIBRARY_NODES: usize = 2_000_000;
const MAX_LIBRARY_VARIABLES: usize = 256;
const MAX_LIBRARY_DATASETS: usize = 1_000;
const MAX_LIBRARY_NAME: usize = 255;
const MAX_LIBRARY_BYTES: usize = 8 << 20;

/// The variable library in `o` or the dictionaries it holds: XML nodes (`/XMLNode`), each with its
/// name (`xmlnode-nodename`), value (`xmlnode-nodevalue`), type (`xmlnode-nodetype`: 1 an element, 2
/// an attribute, 3 text), children (`xmlnode-children`, an array in order) and attributes
/// (`xmlnode-attributes`, a dictionary of attribute nodes). The library is a `variableSets` element
/// holding a `variableSet` (its `locked` attribute `none`, or `variableAndTypes` when the variables
/// are locked), with a `variables` element of `variable` elements (`varName`, and `trait`:
/// `textcontent`, `visibility`, `fileref` or `graphdata`; another trait or none is a variable of
/// unknown kind), and a `sampleDataSets` element (`activeDataSet`, the data set shown) of
/// `sampleDataSet` elements (`dataSetName`). In a data set, the element named after a variable holds
/// its value: `p` elements of text, one paragraph each; a `/Bool` text, whether its objects show; a
/// linked file's URL (`file://`) or path. Graph values aren't read. What is malformed, repeated or
/// past the caps (a declaration, a data set, a value, a setting, a second `variableSet`) is left out
/// and counted; a library past [`MAX_LIBRARY_NODES`] isn't read. Bindings aren't read.
fn variable_library(o: &Obj) -> VariableRead {
    find_variable_sets(o, 0).map_or_else(VariableRead::default, read_variable_sets)
}

/// The `variableSets` XML node in `o` or the dictionaries it holds.
fn find_variable_sets(o: &Obj, depth: usize) -> Option<&Obj> {
    if depth > MAX_DEPTH {
        return None;
    }
    if o.ty == "XMLNode" && element_name(o).as_deref() == Some("variableSets") {
        return Some(o);
    }
    o.items().find_map(|c| find_variable_sets(c, depth + 1))
}

/// A value of an XML node's entry, by the type the editing data gives it.
#[derive(Clone, Debug, PartialEq)]
enum XmlValue {
    /// A `/String` or `/UnicodeString`: its strings joined (a long one is written across lines).
    Text(String),
    /// A `/Bool`.
    Bool(bool),
}

/// The value of entry `key` of the XML node `o`; `None` without it, when it is of another type, or
/// when its text is longer than [`MAX_LIBRARY_BYTES`].
fn xml_value(o: &Obj, key: &str) -> Option<XmlValue> {
    let (V::Name(ty), rest) = o.get(key)?.split_last()? else { return None };
    match ty.as_str() {
        "String" | "UnicodeString" => {
            let len = rest.iter().map(|v| if let V::Str(s) = v { s.len() } else { 0 }).fold(0usize, usize::saturating_add);
            if len > MAX_LIBRARY_BYTES {
                return None;
            }
            let mut bytes = Vec::with_capacity(len);
            for v in rest {
                let V::Str(s) = v else { return None };
                bytes.extend_from_slice(s);
            }
            Some(XmlValue::Text(obj::text(&bytes)))
        }
        "Bool" => match rest {
            [V::Num(n)] if *n == 0.0 => Some(XmlValue::Bool(false)),
            [V::Num(n)] if *n == 1.0 => Some(XmlValue::Bool(true)),
            _ => None,
        },
        _ => None,
    }
}

/// Whether `o` is an XML node of type `ty` (1 an element, 2 an attribute, 3 text).
fn is_node(o: &Obj, ty: f64) -> bool {
    o.ty == "XMLNode" && o.nums("xmlnode-nodetype") == [ty]
}

/// The name of the XML node `o` as the file writes it, prefix and all.
fn node_name(o: &Obj) -> Option<String> {
    match xml_value(o, "xmlnode-nodename")? {
        XmlValue::Text(name) => Some(name),
        XmlValue::Bool(_) => None,
    }
}

/// Whether `o` is an element whose name without its prefix is `name`.
fn is_element(o: &Obj, name: &str) -> bool {
    is_node(o, 1.0) && node_name(o).is_some_and(|n| n.rsplit(':').next() == Some(name))
}

/// The text of the attribute `name` (without its prefix) of the element `o`.
fn xml_attribute(o: &Obj, name: &str) -> Option<String> {
    let attributes = o.obj("xmlnode-attributes").filter(|a| a.ty == "Dictionary")?;
    let node = attributes.items().find(|a| is_node(a, 2.0) && node_name(a).is_some_and(|n| n.rsplit(':').next() == Some(name)))?;
    match xml_value(node, "xmlnode-nodevalue")? {
        XmlValue::Text(text) => Some(text),
        XmlValue::Bool(_) => None,
    }
}

/// A name of the library (a variable's or a data set's): trimmed, not empty and at most
/// [`MAX_LIBRARY_NAME`] characters.
fn library_name(name: Option<String>) -> Option<String> {
    let name = name?.trim().to_string();
    (!name.is_empty() && name.chars().count() <= MAX_LIBRARY_NAME).then_some(name)
}

/// The file path of a linked file's value: a `file://` URL (`file:////Users/…`) as its path, its
/// `%XX` escapes decoded (a URL that names a host other than `localhost` as a network path,
/// `//host/…`), or an absolute path as it is. White space around it isn't part of it.
fn linked_path(text: &str) -> Option<String> {
    let text = text.trim_matches([' ', '\t', '\n', '\r']);
    let drive = |p: &str| matches!(p.as_bytes(), [l, b':', ..] if l.is_ascii_alphabetic());
    let path = match text.strip_prefix("file://") {
        Some(rest) => {
            let rest = rest.strip_prefix("localhost").filter(|r| r.starts_with('/')).unwrap_or(rest);
            if rest.starts_with('/') {
                let p = percent_decoded(rest.trim_start_matches('/'));
                if drive(&p) { p } else { format!("/{p}") }
            } else {
                // A host: the share it names.
                format!("//{}", percent_decoded(rest))
            }
        }
        None => text.to_string(),
    };
    (path.len() > 1 && (path.starts_with('/') || path.starts_with("\\\\") || drive(&path))).then_some(path)
}

/// `s` with `%XX` escapes decoded (a file URL's path); as it is when the decoded bytes aren't UTF-8.
fn percent_decoded(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let mut out = Vec::with_capacity(s.len());
    let mut rest = s.as_bytes();
    while let Some((&b, tail)) = rest.split_first() {
        let hex = tail
            .get(..2)
            .filter(|h| h.iter().all(u8::is_ascii_hexdigit))
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match hex {
            Some(v) if b == b'%' => {
                out.push(v);
                rest = tail.get(2..).unwrap_or_default();
            }
            _ => {
                out.push(b);
                rest = tail;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// The walk through a variable library: the nodes read, the bytes kept, and what was left out.
#[derive(Default)]
struct LibraryWalk {
    nodes: usize,
    bytes: usize,
    skipped: usize,
}

impl LibraryWalk {
    /// The child nodes of the XML node `o`, in order. `None` when they aren't an array, or when the
    /// library has more nodes than [`MAX_LIBRARY_NODES`].
    fn children<'o>(&mut self, o: &'o Obj) -> Option<Vec<&'o Obj>> {
        let list = o.obj("xmlnode-children").filter(|c| c.ty == "Array")?;
        let children: Vec<&Obj> = list.items().collect();
        self.nodes = self.nodes.saturating_add(children.len());
        (self.nodes <= MAX_LIBRARY_NODES).then_some(children)
    }

    /// Whether `len` more bytes of names and values fit in [`MAX_LIBRARY_BYTES`] (they are then counted).
    fn keep(&mut self, len: usize) -> bool {
        let fits = self.bytes.saturating_add(len) <= MAX_LIBRARY_BYTES;
        if fits {
            self.bytes += len;
        }
        fits
    }

    /// Whether a value of `len` bytes would still fit in [`MAX_LIBRARY_BYTES`] (nothing is counted).
    fn room(&self, len: usize) -> bool {
        self.bytes.saturating_add(len) <= MAX_LIBRARY_BYTES
    }

    /// The `variableSet` `set`, into `vars`.
    fn variable_set(&mut self, set: &Obj, vars: &mut Variables) {
        vars.locked = match xml_attribute(set, "locked").as_deref() {
            None | Some("none") => false,
            Some("variableAndTypes") => true,
            Some(_) => {
                self.skipped += 1;
                false
            }
        };
        let Some(children) = self.children(set) else {
            self.skipped += 1;
            return;
        };
        for list in children.iter().filter(|c| is_element(c, "variables")) {
            self.declarations(list, vars);
        }
        for list in children.iter().filter(|c| is_element(c, "sampleDataSets")) {
            self.data_sets(list, vars);
        }
    }

    /// The `variable` elements of the `variables` element `list`, into `vars`.
    fn declarations(&mut self, list: &Obj, vars: &mut Variables) {
        let Some(children) = self.children(list) else {
            self.skipped += 1;
            return;
        };
        for v in children.into_iter().filter(|c| !is_node(c, 3.0)) {
            let name = is_element(v, "variable").then(|| library_name(xml_attribute(v, "varName"))).flatten();
            let Some(name) = name.filter(|n| vars.variable(n).is_none() && vars.variables.len() < MAX_LIBRARY_VARIABLES && self.keep(n.len())) else {
                self.skipped += 1;
                continue;
            };
            let kind = match xml_attribute(v, "trait").as_deref() {
                Some("textcontent") => VariableKind::Text,
                Some("visibility") => VariableKind::Visibility,
                Some("fileref") => VariableKind::Image,
                Some("graphdata") => VariableKind::Graph,
                _ => VariableKind::Unknown,
            };
            vars.variables.push(Variable { name, kind });
        }
    }

    /// The `sampleDataSet` elements of the `sampleDataSets` element `list`, into `vars`, and the
    /// data set it shows.
    fn data_sets(&mut self, list: &Obj, vars: &mut Variables) {
        let Some(children) = self.children(list) else {
            self.skipped += 1;
            return;
        };
        for row in children.into_iter().filter(|c| !is_node(c, 3.0)) {
            let name = is_element(row, "sampleDataSet").then(|| library_name(xml_attribute(row, "dataSetName"))).flatten();
            let Some(name) = name.filter(|n| vars.dataset(n).is_none() && vars.datasets.len() < MAX_LIBRARY_DATASETS && self.keep(n.len())) else {
                self.skipped += 1;
                continue;
            };
            let values = self.values(row, vars);
            vars.datasets.push(DataSet { name, values });
        }
        if let Some(active) = xml_attribute(list, "activeDataSet").and_then(|a| library_name(Some(a))) {
            vars.active_dataset = vars.dataset(&active).map(|_| active);
        }
    }

    /// The values of the `sampleDataSet` element `row`, for the variables of `vars`.
    fn values(&mut self, row: &Obj, vars: &Variables) -> std::collections::BTreeMap<String, DataValue> {
        let mut values = std::collections::BTreeMap::new();
        let Some(children) = self.children(row) else {
            self.skipped += 1;
            return values;
        };
        for el in children.into_iter().filter(|c| !is_node(c, 3.0)) {
            let variable = is_node(el, 1.0).then(|| node_name(el)).flatten().and_then(|n| vars.variable(&n));
            let Some(variable) = variable.filter(|v| !values.contains_key(&v.name)) else {
                self.skipped += 1;
                continue;
            };
            match self.value(el, variable.kind) {
                Ok(Some(value)) => {
                    values.insert(variable.name.clone(), value);
                }
                Ok(None) => {}
                Err(()) => self.skipped += 1,
            }
        }
        values
    }

    /// The value of a variable of `kind` that the element `el` holds: `Ok(None)` when it holds none,
    /// `Err` when it can't be read (graph values aren't read).
    fn value(&mut self, el: &Obj, kind: VariableKind) -> Result<Option<DataValue>, ()> {
        let children = self.children(el).ok_or(())?;
        if children.is_empty() {
            return Ok(None);
        }
        let value = match kind {
            VariableKind::Text => {
                let (mut paragraphs, mut total) = (Vec::new(), 0usize);
                // Text between the paragraphs is white space the XML had.
                for p in children.into_iter().filter(|c| !is_node(c, 3.0)) {
                    if !is_element(p, "p") {
                        return Err(());
                    }
                    let mut paragraph = String::new();
                    for t in self.children(p).ok_or(())? {
                        match (is_node(t, 3.0), xml_value(t, "xmlnode-nodevalue")) {
                            (true, Some(XmlValue::Text(text))) if self.room(total.saturating_add(text.len())) => {
                                total += text.len();
                                paragraph.push_str(&text);
                            }
                            _ => return Err(()),
                        }
                    }
                    // The line break that joins it to the next one.
                    total = total.saturating_add(1);
                    paragraphs.push(paragraph);
                }
                if paragraphs.is_empty() {
                    return Ok(None);
                }
                DataValue::Text(paragraphs.join("\n"))
            }
            VariableKind::Visibility => match children.as_slice() {
                [t] if is_node(t, 3.0) => match xml_value(t, "xmlnode-nodevalue") {
                    Some(XmlValue::Bool(show)) => DataValue::Visible(show),
                    _ => return Err(()),
                },
                _ => return Err(()),
            },
            VariableKind::Image => {
                let mut text = String::new();
                for t in children {
                    match (is_node(t, 3.0), xml_value(t, "xmlnode-nodevalue")) {
                        (true, Some(XmlValue::Text(s))) if self.room(text.len().saturating_add(s.len())) => text.push_str(&s),
                        _ => return Err(()),
                    }
                }
                if text.trim_matches([' ', '\t', '\n', '\r']).is_empty() {
                    return Ok(None);
                }
                DataValue::Image(linked_path(&text).ok_or(())?)
            }
            VariableKind::Graph | VariableKind::Unknown => return Err(()),
        };
        let len = match &value {
            DataValue::Text(s) | DataValue::Image(s) | DataValue::Graph(s) => s.len(),
            DataValue::Visible(_) => 1,
        };
        if self.keep(len) { Ok(Some(value)) } else { Err(()) }
    }
}

/// The library of the `variableSets` element `sets` (see [`variable_library`]).
fn read_variable_sets(sets: &Obj) -> VariableRead {
    let mut walk = LibraryWalk::default();
    let mut vars = Variables::default();
    match walk.children(sets) {
        Some(children) => {
            let (sets, other): (Vec<&Obj>, Vec<&Obj>) = children.into_iter().filter(|c| !is_node(c, 3.0)).partition(|c| is_element(c, "variableSet"));
            // A node that isn't a variable set, and a second set (VectorCraft keeps one set of
            // variables), are left out.
            walk.skipped += other.len() + sets.len().saturating_sub(1);
            if let Some(first) = sets.first() {
                walk.variable_set(first, &mut vars);
            }
        }
        None => walk.skipped += 1,
    }
    if walk.nodes > MAX_LIBRARY_NODES {
        return VariableRead { variables: Variables::default(), skipped: 1, present: true };
    }
    VariableRead { variables: vars, skipped: walk.skipped, present: true }
}
