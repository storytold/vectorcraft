//! Fill and stroke paint (the toolbar proxies and their defaults) and the Transparency panel.

use std::borrow::Cow;

use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Color, GradientPaint, Paint};
use vectorcraft_doc::appearance::{FillLayer, StrokeLayer};
use vectorcraft_doc::{Appearance, AppearanceItem, CharStyle, Document, GraySource, Node, NodeId, NodeKind};
use vectorcraft_tools::builder::{edges, faces, is_live_paint};

use super::appearance::{ItemTarget, edit_items, edit_items_then, item_target, row_targets};
use super::gradient::{item_paint_bounds, place_paint, place_run_paint, run_paint_mut, run_stroke_weight, unplaced};
use super::{
    Affine, CommandSpec, Result, Session, always, bad, bool_or, check_numbers, check_switches, cmd, color_value, has_doc, leaf_targets, ok,
    paint_targets, str_param, switch_or,
};
use crate::{DocState, EngineError};

/// The Live Paint Bucket's tool id. While it is active, a fill or stroke color chosen for no
/// object in particular is the color it paints with ([`apply_paint`]).
pub(crate) const LIVE_PAINT_BUCKET: &str = "livePaintBucket";

/// The Shape Builder's tool id. While it is active, a fill chosen for no object in particular is
/// the current fill only, which merges fill with when Pick Color From is Color Swatches; a stroke
/// chosen then still paints the selection.
const SHAPE_BUILDER: &str = "shapeBuilder";

/// Tool `id`'s id when it paints with the fill (or the stroke, `stroke`) chosen while it is
/// active: the Live Paint Bucket with both, the Shape Builder with the fill.
pub(crate) fn painting_tool_of(id: &str, stroke: bool) -> Option<&'static str> {
    match id {
        LIVE_PAINT_BUCKET => Some(LIVE_PAINT_BUCKET),
        SHAPE_BUILDER if !stroke => Some(SHAPE_BUILDER),
        _ => None,
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "paint.setFill",
            "Fill",
            [],
            None,
            "{color?: \"#rrggbb\"|[r,g,b]|{c,m,y,k}|{gray}|{l,a,b} (CIE Lab; #rrggbbaa and #rgba are refused), none?: true, swatch?: name (a global or spot colour stays linked, so swatch edits recolour it; a tint swatch links to its base at its tint; a gradient swatch is recorded as the gradient's swatch and fits each object, keeping its aspect; the built-in \"[Registration]\" prints on every plate), tint?: 0..100 (% of a global or spot `swatch`; default 100, or a tint swatch's own), gradient?: {kind?: linear|radial|conic|diamond|freeform (conic, also accepted as angle or conical: the stops sweep once around the center `start`, clockwise from the vector; diamond: the stops run from the center `start` out to a square with a corner at `end`), method?: classic|perceptual|hue-clockwise|hue-counterclockwise (as paint.editGradient; default classic), spread?: pad|repeat|reflect (as paint.editGradient; default pad), repeats?: 1..100 (as paint.editGradient; default 1), stops?: [{offset 0..1, color, opacity? 0..1 (or 0..100), midpoint? 0.13..0.87, swatch?: global or spot colour (or tint) swatch the stop links to (its colour comes from the swatch), tint?: 0..100}] (at least 2; default white→black), angle?: deg counter-clockwise (0: the gradient runs left to right), start?: [x,y], end?: [x,y] (the vector in document coordinates, both or neither; type objects keep it in text space), aspect?: % (radial, conic and diamond; a negative one mirrors a conic's sweep, counterclockwise, and is an error for the other kinds; without start/end the gradient is placed on each object's bounds), focal?: [x,y] (radial, with start/end: the focal point, where the first stop sits; an error with a conic or diamond), dither?: bool (linear, radial, conic and diamond fills: drawn with fine noise that hides banding, and as pixels in SVG and PDF exports; strokes ignore it), swatch?: linked gradient swatch name}, item?: appearance item index|null (omitted: the Appearance panel's active item if it is a fill, else the top fill), aligned?: bool (index and item are rows of the first target's stack: each other target edits its equal one, as the Appearance panel does for a mixed selection; with the art of a graph design in a series given first, rows of the art's stack: docs/mcp.md), opacity?: 0..100 (a percentage, as transparency.set's; the opacity of that fill item on each target, in the undo step of the paint, or alone without color, none, swatch or gradient, when it changes neither the defaults nor the recent colors (with New Art Has Basic Appearance off, the next shape copies the first selected object's appearance after any command, its opacity included); with no paint or a solid color, the gradient mesh points picked with the Mesh tool, Direct Selection or the Lasso take it themselves, and with another paint the fill item takes it; with no paint, a target without that fill item is left as it is (a paint given with the opacity creates the item, which then takes it, except for the color a grayscale or 1-bit image is drawn in, which creates none), and an opacity that no target takes is an error that also leaves the paint undone; type characters have no opacity of their own, so an opacity that only characters would take is an error, and so are one with nothing selected and no ids and one for an object selected in a symbol instance), ids?|id?, defaultsOnly?: bool, focus?: true (false keeps the active proxy), keepModel?: false (in a CMYK document, RGB colours and gradient stops are stored as CMYK unless true; Gray stays Gray), target?: \"object\"|\"contents\", span?: \"object\" (default: each target fits a gradient to its own bounds)|\"selection\" (a linear, radial, conic or diamond `gradient` without start/end is placed once on the union of the targets' bounds, so it runs across them)} sets the selection's fill and the default (new art fits a gradient to itself); a solid color recolors only the gradient mesh points picked with the Mesh tool, Direct Selection or the Lasso (unless an `item` index is given or the Appearance panel's active item is a fill); while the Type tool has characters selected (and no ids, id or item are given), the paint goes to those characters only; without `item`, a Live Paint group among the targets (or inside one) takes the fill on its faces only; without `item`, a grayscale or 1-bit image takes a solid color as the color it is drawn in (white pixels transparent; `none` takes it off; an image with an alpha channel takes only a spot color, black or white; a colorized image whose link is missing takes a solid color as its new colorize color, and refuses a process color while its color is a spot color, black or white); with an object of a dynamic symbol instance selected by Direct Selection and no ids, id or item, that object's fill changes for this instance only (solid, none, gradient or pattern). On a path directly in a graph series, the graph also stores the paint of the topmost visible fill when the command wrote that row or changed it, and its opacity when `opacity` wrote that row (exceptions: docs/mcp.md); on a graph label, the graph stores the paint for every label of its kind, as a character fill when the characters took it and with the label's rows when a row did; on a graph's axis line, tick mark or radar ring, its rows for every line of its kind. While the Live Paint Bucket or the Shape Builder is the active tool and no ids, id or item are given (a null ids or id counts as none), or `defaultsOnly: true` is given, only the default changes (the color the bucket paints with, and the fill the Shape Builder's merges take with Pick Color From Color Swatches) and the selection keeps its paint; `defaultsOnly` with ids, id or item is refused, and the journal records a change either tool made this way with `defaultsOnly: true`, so a replay with another tool active changes the default too → {objects: how many objects took the paint (the objects inside a group, unless an appearance item of the group's own is targeted; 0 when only the default changed), or with an opacity alone how many took the opacity}",
            has_doc,
            |s, p| set_paint(s, p, true)
        ),
        cmd!(
            "paint.setStroke",
            "Stroke",
            [],
            None,
            "{…as paint.setFill, item?: the stroke item to set, aligned?: bool (as paint.setFill), opacity?: 0..100 (the opacity of that stroke item, as paint.setFill's for the fill item)} same as paint.setFill, for the stroke (while the Shape Builder is the active tool, the selection still takes the stroke); without `item`, a Live Paint group among the targets (or inside one) takes the stroke on its edges only; with an object of a dynamic symbol instance selected by Direct Selection and no ids, id or item, that object's stroke changes for this instance only (solid, none, gradient or pattern). On a path directly in a graph series, the graph also stores the paint and weight of the topmost visible stroke when the command wrote that row or changed it, and its opacity when `opacity` wrote that row (exceptions: docs/mcp.md); on a graph label, as paint.setFill",
            has_doc,
            |s, p| { set_paint(s, p, false) }
        ),
        cmd!(
            "paint.swap",
            "Swap Fill and Stroke",
            [],
            Some("Shift+X"),
            "{ids?|id?, defaultsOnly?: bool} swap the fill and stroke of the selection (type too) and of the defaults. A Live Paint group's faces take the stroke its edges share and its edges take the fill its faces share, a gradient fitted to each face or edge; a group whose faces' fills or edges' strokes differ (a freeform gradient counts as differing), or that has no face or no edge, keeps its paint. On a path directly in a graph series, the graph also stores the swapped paints and the stroke weight, and on a graph label the swapped paints of its characters for every label of its kind, and on a graph's axis line, tick mark or radar ring what the swap changes on its rows for every line of its kind (exceptions: docs/mcp.md). While the Live Paint Bucket or the Shape Builder is the active tool and no ids or id are given (a null one counts as none), or `defaultsOnly: true` is given, only the defaults change (with the Shape Builder, the default fill takes the stroke the Stroke proxy shows, the selection's)",
            has_doc,
            swap
        ),
        cmd!(
            "paint.default",
            "Default Fill and Stroke",
            [],
            Some("D"),
            "{ids?|id?, defaultsOnly?: bool} white fill and 1 pt black stroke for the selection and the defaults (in a document whose Default Graphic Style differs from the shipped one, as a New Document profile can redefine it: that style's fills, strokes and effects, without its opacity or a link to it); type gets black fill and no stroke. A Live Paint group's faces get no fill and its edges get a 0.5 pt black stroke, in place of their whole appearance and also when the Default Graphic Style is redefined, and with only Live Paint groups selected (or in ids) the defaults become no fill and a 0.5 pt black stroke (with New Art Has Basic Appearance off, new art then takes the selection's appearance, as after any command). On a path directly in a graph series, the graph also stores the paint, weight, opacity and blend mode of the topmost visible fill and stroke, and on a graph label the black fill and no stroke of its characters for every label of its kind, and on a graph's axis line, tick mark or radar ring its rows for every line of its kind (exceptions: docs/mcp.md). While the Live Paint Bucket or the Shape Builder is the active tool and no ids or id are given (a null one counts as none), or `defaultsOnly: true` is given, only the defaults change",
            has_doc,
            default_paint
        ),
        cmd!(
            "paint.toggleActive",
            "Toggle Fill/Stroke Focus",
            [],
            Some("X"),
            "{fill?: bool (true: bring the Fill proxy forward, false: the Stroke proxy; omitted: swap which is in front)} → {fillActive}",
            always,
            |s, p| {
                s.fill_active = switch_or(p, "fill", !s.fill_active, "paint.toggleActive")?;
                Ok(json!({ "fillActive": s.fill_active }))
            }
        ),
        cmd!(
            "paint.none",
            "None",
            [],
            Some("/"),
            "{defaultsOnly?: bool} set the active proxy (fill or stroke) to None, as paint.setFill or paint.setStroke with none does (while the Live Paint Bucket, or for a fill the Shape Builder, is the active tool, or with `defaultsOnly: true`, only the default changes) → {objects}",
            has_doc,
            |s, p| {
                let f = s.fill_active;
                let only = switch_or(p, "defaultsOnly", false, "paint.none")?;
                set_paint(s, &with_defaults_only(json!({"none": true}), only), f)
            }
        ),
        cmd!(
            "transparency.set",
            "Transparency",
            ["Window", "Transparency"],
            None,
            "{ids?|id?, opacity?: 0..100, blend?: name, isolate?, knockout?: \"on\"|\"off\"|\"neutral\"|bool (true = on, false = neutral), knockoutShape?: bool, item?: appearance item index|null, target?: \"object\"|\"contents\", aligned?: bool (index and item are rows of the first target's stack: each other target edits its equal one, as the Appearance panel does for a mixed selection; with the art of a graph design in a series given first, rows of the art's stack: docs/mcp.md)} for `ids`, the selection, or the object whose opacity mask is being edited; opacity and blend go to the targeted fill/stroke item (omitted: the Appearance panel's active item, else the objects; when every target is a gradient mesh with points picked by the Mesh tool, Direct Selection or the Lasso, those points take the opacity themselves). With an object of a dynamic symbol instance selected by Direct Selection and no ids, id or item, opacity and blend change that object for this instance only (isolate, knockout, knockoutShape, aligned and target are refused). On a path directly in a graph series (for a fill or stroke item, a series group's id stands for those paths), the graph also stores the opacity and blend mode of the object and of the topmost visible fill and stroke, and on a series group without an item, the group's own opacity, blend mode, isolate, knockout and knockoutShape (exceptions: docs/mcp.md); on a graph label, or a graph's axis line, tick mark or radar ring (for a fill or stroke item, a graph's Axes group stands for the lines and labels directly in it, its Legend group for the labels; when their rows differ, `item` needs `aligned` and is a row of the backmost one), the graph stores that object's opacity and blend mode for every label or line of its kind, and with `item` its topmost visible fill and stroke rows too",
            has_doc,
            transparency
        ),
    ]
}

/// Commands of the Fill/Stroke proxies, registered at the end of the command list.
pub fn proxy_specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "paint.invert",
            "Invert",
            [],
            None,
            "{stroke?: bool (default: the active proxy), ids?|id?, defaultsOnly?: bool, item?: appearance item index|null, aligned?: bool (as paint.setFill), target?: \"object\"|\"contents\"} invert the active proxy's colours of the selection (or ids; groups recolour their contents and, except with target \"contents\", their own fills and strokes; a compound path's member stands for its compound path; a Live Paint group recolors its faces' fills or its edges' strokes) keeping each colour's model, as edit.colors.invert does; the Appearance panel's active fill/stroke item (appearance.setActiveItem) is inverted instead when it is of that kind and no ids are given; with an object of a dynamic symbol instance selected by Direct Selection and no ids, id or item, that object's solid color changes for this instance only, and a gradient or pattern, or type whose characters differ in color, is refused; with nothing selected, or while the Live Paint Bucket, or for a fill the Shape Builder, is the active tool and no ids, id or item are given (a null ids or id counts as none), or `defaultsOnly: true` is given, invert the default. On a path directly in a graph series, the graph also stores the new color of the topmost visible fill or stroke, and on a graph label the new color of its characters and rows for every label of its kind, and on a graph's axis line, tick mark or radar ring its rows for every line of its kind (docs/mcp.md); no undo step when no color changes → {changed}",
            has_doc,
            |s, p| proxy_recolor(s, p, "paint.invert", "Invert", &super::colorcmds::invert)
        ),
        cmd!(
            "paint.complement",
            "Complement",
            [],
            None,
            "{stroke?: bool (default: the active proxy), ids?|id?, defaultsOnly?: bool, item?: appearance item index|null, aligned?: bool (as paint.setFill), target?: \"object\"|\"contents\"} replace the active proxy's colours by their complements ((highest + lowest) − each component, over RGB or CMY; grey unchanged) keeping each colour's model; a Live Paint group recolors its faces' fills or its edges' strokes as with paint.invert; honours the Appearance panel's active item and a selected object of a dynamic symbol instance as paint.invert does; with nothing selected, or while the Live Paint Bucket, or for a fill the Shape Builder, is the active tool and no ids, id or item are given (a null ids or id counts as none), or `defaultsOnly: true` is given, the default; no undo step when no color changes → {changed}",
            has_doc,
            |s, p| proxy_recolor(s, p, "paint.complement", "Complement", &|c: Color| c.complement_keep_model())
        ),
        cmd!(
            "paint.lastColor",
            "Apply Last Color",
            [],
            Some(","),
            "{stroke?: bool (default: the active proxy), ids?|id?, defaultsOnly?: bool, item?: appearance item index|null, aligned?: bool (as paint.setFill), target?: \"object\"|\"contents\"} apply the last solid colour used (paint.recent lastColor) to the active proxy of the selection and the default (while the Live Paint Bucket, or for a fill the Shape Builder, is the active tool and no ids, id or item are given (a null ids or id counts as none), or `defaultsOnly: true` is given, to the default only) → {objects} (as paint.setFill)",
            has_doc,
            |s, p| {
                let paint = Paint::solid(s.last_solid);
                apply_to_proxy(s, p, paint, "paint.lastColor")
            }
        ),
        cmd!(
            "paint.lastGradient",
            "Apply Last Gradient",
            [],
            Some("."),
            "{stroke?: bool (default: the active proxy), ids?|id?, defaultsOnly?: bool, item?: appearance item index|null, aligned?: bool (as paint.setFill), target?: \"object\"|\"contents\"} apply the last gradient used (paint.recent lastGradient), fitted to each object, to the active proxy of the selection and the default (while the Live Paint Bucket, or for a fill the Shape Builder, is the active tool and no ids, id or item are given (a null ids or id counts as none), or `defaultsOnly: true` is given, to the default only) → {objects} (as paint.setFill)",
            has_doc,
            |s, p| {
                let paint = super::gradient::unplaced(&Paint::Gradient(Box::new(s.last_gradient.clone())));
                apply_to_proxy(s, p, paint, "paint.lastGradient")
            }
        ),
        cmd!(
            query "paint.recent",
            "Recent Colors",
            [],
            None,
            "{} → {colors: [{hex, color}] newest first, at most 30, each once, kept across documents for the session (fed by every paint command and the eyedropper), lastColor: {hex, color}, lastGradient: gradient paint}",
            always,
            |s, _| {
                let c = |c: &Color| json!({"hex": c.to_hex(), "color": c});
                Ok(json!({
                    "colors": s.recent_colors.iter().map(c).collect::<Vec<_>>(),
                    "lastColor": c(&s.last_solid),
                    "lastGradient": s.last_gradient,
                }))
            }
        ),
        cmd!(
            query "paint.proxies",
            "Fill and Stroke",
            [],
            None,
            "{} → {fill, stroke: the paints the Fill/Stroke proxies show (the Appearance panel's active item for the proxy of its kind, else the first selected object's, a group's first painted object's, a Live Paint group's first face's fill and first edge's stroke, type: its first run's; else the defaults for new art; when that object is a gradient mesh with points picked by the Mesh tool, Direct Selection or the Lasso and no fill item is active, the fill is its first picked point's color), fillActive, fillMixed, strokeMixed: the selected objects' (or a selected group's contents') fills (strokes) differ, a Live Paint group counting with its faces' fills and its edges' strokes, shown as a \"?\" proxy (a gradient mesh with points picked counts with their colors), fillOpacity, strokeOpacity: 0..100, the opacity of the fill item that paint.setFill {opacity} sets (the stroke item that paint.setStroke {opacity} sets), from the first selected object (or painted object in a selected group; a selected path of a compound path stands for its compound, and a Live Paint group for its faces' fills and its edges' strokes) that has one (a gradient mesh's first picked point's), or null with nothing selected (the defaults for new art), for an object selected in a symbol instance, while the Live Paint Bucket is the active tool (for fillOpacity, also the Shape Builder), while the Type tool has characters selected, and when only type characters or objects without that item are selected}",
            always,
            proxies
        ),
    ]
}

impl Session {
    /// How many colors Recent Colors keeps. The Session holds the list, so it stays the same when
    /// another document is opened or made active.
    pub const RECENT_MAX: usize = 30;

    /// Remember an applied paint: a solid colour becomes the last colour and the newest recent
    /// colour, a gradient the last gradient. During a live preview this waits for the commit.
    pub(crate) fn remember_paint(&mut self, p: &Paint) {
        if self.in_interaction() {
            self.pending_paint = Some(p.clone());
        } else {
            self.remember_paint_now(p);
        }
    }

    /// The fill and stroke the Fill/Stroke proxies show: the Appearance panel's active item (in the
    /// first selected object's own stack) for the proxy of its kind, else the first selected
    /// object's (a group's first painted object's; a Live Paint group's first face's fill and first
    /// edge's stroke, or the next selected object's when it has no face or edge; type: its first
    /// run's), else the defaults for new art. When the object whose fill the proxy shows is a
    /// gradient mesh with points picked ([`Session::fill_mesh_points`]), the fill is its first
    /// picked point's color, which a solid fill color changes.
    pub fn proxy_paints(&self) -> (Paint, Paint) {
        let points = self.fill_mesh_points();
        let show = |stroke: bool| match self.proxy_source(stroke) {
            Some((n, None)) if !stroke && let Some(c) = picked_colors(&n, &points).first() => Paint::solid(*c),
            Some((n, item)) => proxy_paint(&n, stroke, item),
            None if stroke => self.paint.stroke.clone(),
            None => self.paint.fill.clone(),
        };
        (show(false), show(true))
    }

    /// The object whose fill (or stroke) the proxy shows, with the Appearance panel's active item
    /// when it stands for it (see [`Session::proxy_paints`]); None: the defaults for new art. With
    /// an object of a dynamic symbol instance selected ([`DocState::selected_symbol_part`]), that
    /// object as the instance draws it. None while the active tool paints with the chosen fill (or
    /// stroke), or another tool works for it ([`Session::tool_paints`]): the proxy shows the color
    /// it paints with.
    pub(crate) fn proxy_source(&self, stroke: bool) -> Option<(Cow<'_, Node>, Option<usize>)> {
        if self.tool_paints(stroke) {
            return None;
        }
        let st = self.active()?;
        if let Some(part) = st.selected_symbol_part() {
            return Some((Cow::Owned(part), None));
        }
        let first = st.doc.node(*st.selection.subjects().first()?)?;
        let item = self.appearance_item();
        // Only an active Appearance row is shown here: it is a row of the first selected object.
        if first.appearance.item_of_kind(item, !stroke).is_some() {
            return Some((Cow::Borrowed(first), item));
        }
        // A Live Paint group without faces (or edges) shows the next selected object's paint; any other
        // object without a painted object shows the defaults for new art. A face or edge selected with
        // its Live Paint group counts with the group, wherever it is in the selection. A targeted
        // layer stands for its selected editable art, which the paint commands edit
        // (ItemTarget::targets), not for a locked or hidden object in it.
        let covered = st.selection_live_paint_covered(&st.selection.objects);
        for id in st.selection.objects.iter().filter(|id| !covered.contains(*id)) {
            let Some(n) = st.doc.node(*id) else { continue };
            let mut nodes = vec![];
            painted(n, true, stroke, &mut nodes);
            if let Some(n) = nodes.first() {
                return Some((Cow::Borrowed(*n), None));
            }
            if !is_live_paint(n) {
                return None;
            }
        }
        None
    }

    /// The opacity (0..1) of the fill that `paint.setFill {opacity}` sets (or of the stroke that
    /// `paint.setStroke {opacity}` sets): the item's, or for a gradient mesh with points picked the
    /// first picked point's. The rows the command would edit are read in order (a group without an
    /// active Appearance row stands for its contents), and the first that exists answers. `None`
    /// with nothing selected, for an object selected in a symbol instance, while the active tool
    /// paints with the chosen fill (or stroke), or another tool works for it ([`Session::tool_paints`]), while
    /// the Type tool has characters selected, and when only type
    /// characters, objects without that item or the art of graph designs in a graph series are
    /// selected.
    pub fn proxy_opacity(&self, stroke: bool) -> Option<f32> {
        let st = self.active()?;
        // Such a tool takes a fill or stroke chosen as the color it paints with, and the selection
        // keeps its own (for_tool).
        if st.selection.symbol_part.is_some() || self.tool_paints(stroke) {
            return None;
        }
        // The command would give the opacity to the characters, which have none of their own.
        if super::textedit::selected_range(self).is_some_and(|(_, a, b)| a != b) {
            return None;
        }
        let item = self.appearance_item();
        let of = |n: &Node, item: Option<usize>| -> Option<f32> {
            if item.is_none() && matches!(n.kind, NodeKind::Text(_)) {
                return None;
            }
            if !stroke
                && item.is_none()
                && let NodeKind::Mesh(m) = &n.kind
                && let Some((_, points)) = picked_mesh_points(self, &[n.id]).first()
            {
                return points.first().and_then(|i| m.points.get(*i)).map(|q| q.opacity);
            }
            if stroke { n.appearance.stroke_at(item).map(|s| s.opacity) } else { n.appearance.fill_at(item).map(|f| f.opacity) }
        };
        let subjects = st.selection.subjects();
        // The command leaves out the art of a graph design in a series, which the graph draws again
        // from the design, and a series group stands for the paths directly in it
        // (graph::paint_targets).
        let editable = |id: &&NodeId| !super::graph::is_design_art(&st.doc, **id);
        // As the command picks its rows (ItemTarget::of_kind): the active item when it is a row of
        // this kind in the first selected object's stack, read in the stacks the command edits;
        // else the topmost one of the painted leaves of the selected objects (a group's
        // contents; for a targeted layer, its editable art, ItemTarget::targets). A row of a
        // graph design's art selected first stands for nothing (item_target_at).
        let first = subjects.first().filter(editable).and_then(|id| st.doc.node(*id));
        let active = item.filter(|i| first.and_then(|n| n.appearance.items.get(*i)).is_some_and(|it| it.is_fill() == !stroke));
        if let Some(index) = active {
            // The objects whose own stacks the command edits (appearance_targets: a selected
            // compound-path member stands for its compound, and an object inside another selected
            // one is left out), with the one whose stack the row belongs to first.
            // Each target reads the row the command edits there (ItemTarget::per_target and
            // ItemTarget::resolve): the row equal to the active one, else its topmost of the kind.
            let row = ItemTarget::Item { index, explicit: false, aligned: true };
            let mut roots = super::appearance::appearance_targets(self, &Value::Null).ok()?;
            let anchor = row.anchor(self, &roots);
            if let Some(head) = roots.get_mut(..=anchor) {
                head.rotate_right(1);
            }
            let targets: Vec<NodeId> = super::graph::series_group_marks_where(&st.doc, &roots, |n| n.extras().series_index.is_some())
                .into_iter()
                .filter(|id| editable(&id))
                .collect();
            return targets.iter().zip(row.each_target(self, &targets)).find_map(|(id, at)| {
                let n = st.doc.node(*id)?;
                of(n, at?.resolve(&n.appearance, !stroke, "paint.proxies").ok()?)
            });
        }
        // A Live Paint group stands for its faces in a fill edit and for its edges in a stroke edit
        // (ItemTarget::targets_for).
        let leaves = super::leaf_targets(self, &st.selection.objects).ok()?;
        let skipped = live_paint_parts_skipped(&st.doc, &st.selection.objects, !stroke);
        leaves.iter().filter(editable).filter(|id| !skipped.contains(*id)).filter_map(|id| st.doc.node(*id)).find_map(|n| of(n, None))
    }

    pub(crate) fn remember_paint_now(&mut self, p: &Paint) {
        match p {
            Paint::Solid { color, .. } => {
                self.last_solid = *color;
                self.recent_colors.retain(|c| c != color);
                self.recent_colors.insert(0, *color);
                self.recent_colors.truncate(Self::RECENT_MAX);
            }
            Paint::Gradient(g) => self.last_gradient = (**g).clone(),
            Paint::None | Paint::Pattern { .. } => {}
        }
    }
}

/// The fill or stroke a proxy shows for `n` ([`Node::proxy_paint_only`]: fill or stroke `item` when
/// it is of that kind, type's first run, else the topmost one).
pub(crate) fn proxy_paint(n: &Node, stroke: bool, item: Option<usize>) -> Paint {
    match n.proxy_paint_only(stroke, item) {
        Some(p) => p.clone(),
        // Type without runs shows the default character style.
        None if matches!(n.kind, NodeKind::Text(_)) => {
            let st = CharStyle::default();
            if stroke { st.stroke } else { st.fill }
        }
        None => Paint::None,
    }
}

/// Do two paints look the same in a proxy? Solid colours compare without their swatch link and
/// gradients without their per-object geometry.
fn same_in_proxy(a: &Paint, b: &Paint) -> bool {
    match (a, b) {
        (Paint::Solid { color: x, .. }, Paint::Solid { color: y, .. }) => x == y,
        (Paint::Gradient(x), Paint::Gradient(y)) => x.gradient == y.gradient,
        _ => a == b,
    }
}

/// The objects whose paints the proxies show for a selected `n`, the ones the paint commands
/// change ([`leaf_targets`]): groups and layers stand for their contents (and so do blends and
/// other containers inside them), a compound path for its parts. A Live Paint group stands for its
/// faces in the fill proxy (`stroke` false) and for its edges in the stroke proxy, the parts a fill
/// or a stroke given to it changes ([`live_paint_parts_skipped`]).
pub(crate) fn painted<'a>(n: &'a Node, top: bool, stroke: bool, out: &mut Vec<&'a Node>) {
    // A ruler guide has no paints.
    if n.is_ruler_guide() {
        return;
    }
    if is_live_paint(n) {
        out.extend(if stroke { edges(n) } else { faces(n) }.into_iter().map(|part| &**part));
        return;
    }
    let expand = match n.kind {
        NodeKind::Group { .. } | NodeKind::Layer { .. } => true,
        NodeKind::Compound { .. } => false,
        _ => !top && n.is_container(),
    };
    if expand {
        n.children().into_iter().flatten().for_each(|c| painted(c, false, stroke, out));
    } else {
        out.push(n);
    }
}

#[cfg(test)]
thread_local! {
    /// Tests: how many fills [`SelectionPaints`] read or took out on this thread.
    pub(crate) static PAINTS_READ: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The objects that reading objects `ids` meets, one item each ([`SelectionPaints::reads_no_more_than`]):
/// each id, and the contents of a group, a layer, a Live Paint group or a container inside a group,
/// as [`painted`] expands them. An id with no object and a ruler guide count once each.
struct Reads<'a> {
    doc: &'a Document,
    ids: std::slice::Iter<'a, NodeId>,
    /// The contents not counted yet of the objects met, innermost last.
    left: Vec<std::slice::Iter<'a, std::sync::Arc<Node>>>,
}

impl<'a> Reads<'a> {
    fn new(doc: &'a Document, ids: &'a [NodeId]) -> Self {
        Self { doc, ids: ids.iter(), left: vec![] }
    }
}

impl Iterator for Reads<'_> {
    type Item = ();

    fn next(&mut self) -> Option<()> {
        loop {
            let (n, top) = match self.left.last_mut() {
                Some(contents) => match contents.next() {
                    Some(c) => (&**c, false),
                    None => {
                        self.left.pop();
                        continue;
                    }
                },
                None => match self.doc.node(*self.ids.next()?) {
                    Some(n) => (n, true),
                    None => return Some(()),
                },
            };
            // A ruler guide has no paints, but reading meets it, so it counts once.
            if n.is_ruler_guide() {
                return Some(());
            }
            // A Live Paint group is a group too, so it expands without looking for its parts.
            let expand = match n.kind {
                NodeKind::Group { .. } | NodeKind::Layer { .. } => true,
                NodeKind::Compound { .. } => false,
                _ => !top && n.is_container(),
            };
            if let Some(contents) = n.children().filter(|_| expand) {
                self.left.push(contents.iter());
            }
            return Some(());
        }
    }
}

/// What the proxies read from selected objects: the first fill, stroke and stroke weight met, and
/// whether another one differs ([`DocState::proxy_mixed`] without an active Appearance item or
/// picked mesh points, and [`super::stroke::StrokeMixed::weight`]), and whether an object lies in a
/// Live Paint group or may hold a gradient mesh. The objects added to a selection are read with
/// [`SelectionPaints::add`], and the objects taken out of it with [`SelectionPaints::remove`].
#[derive(Clone, Debug, Default)]
pub(crate) struct SelectionPaints {
    fill: Tally<Paint>,
    stroke: Tally<Paint>,
    weight: Tally<f64>,
    /// The objects that are a face or edge of a Live Paint group, or lie inside one
    /// ([`live_paint_covered`]).
    in_live_paint: usize,
    /// The objects that are a gradient mesh or a container ([`Session::fill_mesh_points`]).
    may_hold_mesh: usize,
}

/// The first value met, how many of the values met are the same as it, and how many were met.
#[derive(Clone, Debug)]
struct Tally<T> {
    first: Option<T>,
    same: usize,
    all: usize,
}

impl<T> Default for Tally<T> {
    fn default() -> Self {
        Self { first: None, same: 0, all: 0 }
    }
}

impl<T> Tally<T> {
    fn add(&mut self, value: T, same: impl Fn(&T, &T) -> bool) {
        self.all += 1;
        match &self.first {
            Some(f) if same(f, &value) => self.same += 1,
            Some(_) => {}
            None => (self.first, self.same) = (Some(value), 1),
        }
    }
    /// Take out a value met before. → false when the values left must be met again to know which
    /// differ: every value the same as the first one is gone, and others are left.
    fn remove(&mut self, value: &T, same: impl Fn(&T, &T) -> bool) -> bool {
        let Some(f) = &self.first else { return false };
        if self.all == 0 {
            return false;
        }
        self.all -= 1;
        if same(f, value) {
            self.same = self.same.saturating_sub(1);
        }
        if self.same == 0 {
            self.first = None;
            return self.all == 0;
        }
        true
    }
    fn mixed(&self) -> bool {
        self.same < self.all
    }
}

impl SelectionPaints {
    /// Read objects `ids` of `doc`.
    pub(crate) fn of(doc: &Document, ids: &[NodeId]) -> Self {
        let mut p = Self::default();
        p.add(doc, ids);
        p
    }

    /// Do the fills differ?
    pub(crate) fn fill_mixed(&self) -> bool {
        self.fill.mixed()
    }

    /// Do the strokes differ?
    pub(crate) fn stroke_mixed(&self) -> bool {
        self.stroke.mixed()
    }

    /// Do the stroke weights differ?
    pub(crate) fn weight_mixed(&self) -> bool {
        self.weight.mixed()
    }

    /// Is an object a face or edge of a Live Paint group, or inside one?
    pub(crate) fn in_live_paint(&self) -> bool {
        self.in_live_paint > 0
    }

    /// Is an object a gradient mesh or a container?
    pub(crate) fn may_hold_mesh(&self) -> bool {
        self.may_hold_mesh > 0
    }

    /// Does reading objects `ids` of `doc` meet no more objects than reading objects `others`?
    /// Each id counts once, and a group, a layer, a Live Paint group or a container inside a group
    /// also counts its contents, as [`painted`] expands them. The two counts go up one object at a
    /// time in turn and stop when either side runs out, so the check meets at most twice as many
    /// objects as the smaller side holds.
    pub(crate) fn reads_no_more_than(doc: &Document, ids: &[NodeId], others: &[NodeId]) -> bool {
        let (mut these, mut those) = (Reads::new(doc, ids), Reads::new(doc, others));
        loop {
            if these.next().is_none() {
                return true;
            }
            if those.next().is_none() {
                return false;
            }
        }
    }

    /// Read objects `ids` of `doc` too: ids that are not read yet, each named once.
    pub(crate) fn add(&mut self, doc: &Document, ids: &[NodeId]) {
        self.each(doc, ids, true);
    }

    /// Take out objects `ids` of `doc`, read before in the same document: ids that are read, each
    /// named once. → false when the objects left must be read again ([`Tally::remove`]); the
    /// paints are then wrong.
    pub(crate) fn remove(&mut self, doc: &Document, ids: &[NodeId]) -> bool {
        self.each(doc, ids, false)
    }

    /// [`Self::add`] (`add`) or [`Self::remove`].
    fn each(&mut self, doc: &Document, ids: &[NodeId], add: bool) -> bool {
        let mut ok = true;
        let mut meet = |tally: &mut Tally<Paint>, value: Paint| {
            if add {
                tally.add(value, same_in_proxy);
            } else {
                ok &= tally.remove(&value, same_in_proxy);
            }
        };
        // As in `proxy_mixed_with`: every node with one of the ids, in tree order.
        let (mut fills, mut strokes) = (vec![], vec![]);
        for n in doc.nodes_in_order(ids.iter().copied()) {
            painted(n, true, false, &mut fills);
            painted(n, true, true, &mut strokes);
        }
        #[cfg(test)]
        PAINTS_READ.with(|c| c.set(c.get() + fills.len()));
        for n in fills {
            meet(&mut self.fill, proxy_paint(n, false, None));
        }
        for n in strokes {
            meet(&mut self.stroke, proxy_paint(n, true, None));
        }
        // As in `stroke_mixed`: the node each id names.
        let step = |count: &mut usize, yes: bool| {
            if yes {
                *count = if add { *count + 1 } else { count.saturating_sub(1) };
            }
        };
        let mut seen = std::collections::HashSet::new();
        for id in ids.iter().filter(|id| seen.insert(**id)) {
            let Some(n) = doc.node(*id) else { continue };
            step(&mut self.in_live_paint, in_live_paint(doc, *id));
            step(&mut self.may_hold_mesh, matches!(n.kind, NodeKind::Mesh(_)) || n.is_container());
            let mut nodes = vec![];
            painted(n, true, true, &mut nodes);
            for n in nodes {
                let widths: Vec<f64> = match &n.kind {
                    NodeKind::Text(t) => t.runs.iter().map(|r| r.style.stroke_layer().width).collect(),
                    _ => n.appearance.stroke().map(|s| s.width).into_iter().collect(),
                };
                for w in widths {
                    if add {
                        self.weight.add(w, |a, b| a == b);
                    } else {
                        ok &= self.weight.remove(&w, |a, b| a == b);
                    }
                }
            }
        }
        ok
    }
}

impl DocState {
    /// Whether the selected objects' fills and strokes differ as the proxies show them (a "?"
    /// proxy; a group whose contents differ counts). `item` (the Appearance panel's active item)
    /// stands for the proxy of its kind, as in [`Session::proxy_paints`]: that fill or stroke of
    /// each selected object's own stack is compared. The selected objects are found through the id
    /// cache ([`vectorcraft_doc::Document::nodes_in_order`]); a large selection takes one walk
    /// of the document, so callers that ask every frame cache it by revision.
    pub fn proxy_mixed(&self, item: Option<usize>) -> (bool, bool) {
        self.proxy_mixed_with(item, &[])
    }

    /// [`DocState::proxy_mixed`] with the gradient mesh points a solid fill color recolors
    /// (`points`, see [`Session::fill_mesh_points`]): a mesh among them counts with the colors of
    /// its picked points instead of its fill.
    fn proxy_mixed_with(&self, item: Option<usize>, points: &[(NodeId, Vec<usize>)]) -> (bool, bool) {
        if self.selection.is_empty() {
            return (false, false);
        }
        if self.selection.symbol_part.is_some() {
            return (false, false);
        }
        // Looking the first object up builds the id cache when there is none.
        let first = self.selection.objects.first().and_then(|id| self.doc.node(*id));
        let (mut selected, mut fills, mut strokes) = (vec![], vec![], vec![]);
        // The faces and edges of the selected Live Paint groups (or of those inside selected groups): selected as well,
        // they count with their group's.
        let covered = self.selection_live_paint_covered(&self.selection.objects);
        // Every node with a selected id, in tree order: a repeated id (a damaged file) counts at each
        // place it appears.
        for n in self.doc.nodes_in_order(self.selection.objects.iter().copied()) {
            #[cfg(test)]
            tests::note_gathered();
            selected.push(n);
            if covered.contains(&n.id) {
                continue;
            }
            painted(n, true, false, &mut fills);
            painted(n, true, true, &mut strokes);
        }
        let differ = |stroke: bool| {
            let item = first.and_then(|f| f.appearance.item_of_kind(item, !stroke));
            let set = if item.is_some() {
                &selected
            } else if stroke {
                &strokes
            } else {
                &fills
            };
            let mut paints = set.iter().flat_map(|n| match picked_colors(n, if stroke || item.is_some() { &[] } else { points }) {
                c if c.is_empty() => vec![proxy_paint(n, stroke, item)],
                c => c.into_iter().map(Paint::solid).collect(),
            });
            let Some(p0) = paints.next() else { return false };
            paints.any(|p| !same_in_proxy(&p0, &p))
        };
        (differ(false), differ(true))
    }

    /// The object of a dynamic symbol instance's art selected with Direct Selection
    /// ([`vectorcraft_doc::Selection::symbol_part`]), with the instance's own appearance changes
    /// applied, in the symbol's coordinates. `None` without one.
    pub(crate) fn selected_symbol_part(&self) -> Option<Node> {
        let sp = self.selection.symbol_part.as_ref()?;
        let inst = self.doc.node(sp.instance)?;
        let NodeKind::SymbolInstance { symbol, .. } = &inst.kind else { return None };
        let sym = self.doc.symbols.iter().find(|s| s.name == *symbol)?;
        let art = vectorcraft_doc::symbol_overrides::dynamic_art(&sym.art, inst);
        vectorcraft_doc::symbol_overrides::part(&art, &sp.path).cloned()
    }
}

/// The id of tool `t`, or of the tool its armed pick switches back to, when that tool paints with
/// the chosen fill or stroke ([`painting_tool_of`]).
fn painter_of(t: &dyn vectorcraft_tools::Tool) -> Option<&'static str> {
    let of = |id: &str| painting_tool_of(id, false).or_else(|| painting_tool_of(id, true));
    of(t.id()).or_else(|| t.returns_to().and_then(of))
}

impl Session {
    /// The tool that lent the active tool (a Cmd+click) when it paints with the chosen colors, or
    /// the Eyedropper armed for such a tool did ([`painter_of`]).
    pub(crate) fn lent_by_painter(&self) -> Option<&'static str> {
        self.lender.as_deref().and_then(painter_of)
    }

    /// Do the fill proxy (the stroke proxy, with `stroke`), and the fill (stroke) commands that
    /// name no objects, stand for the color the active tool paints with ([`painting_tool_of`]: the
    /// Live Paint Bucket's fill and stroke, the Shape Builder's fill)? They do while such a tool is
    /// active, while a Cmd+click with it uses the Selection tool, and while the Eyedropper samples
    /// into a stop of its gradient for the Gradient panel's eyedropper, up to and including the
    /// sample its click takes before it switches back to that tool.
    pub fn tool_paints(&self, stroke: bool) -> bool {
        self.painting_tool(stroke).is_some()
    }

    /// The id of the tool whose fill (or stroke, `stroke`) the proxy stands for
    /// ([`Session::tool_paints`]); None when it stands for the selection's.
    pub(crate) fn painting_tool(&self, stroke: bool) -> Option<&'static str> {
        let of =
            |t: &dyn vectorcraft_tools::Tool| painting_tool_of(t.id(), stroke).or_else(|| t.returns_to().and_then(|id| painting_tool_of(id, stroke)));
        // A Cmd+click lends the Selection tool to such a tool, or to the Eyedropper armed for it.
        self.back_to_painter
            .and_then(|id| painting_tool_of(id, stroke))
            .or_else(|| of(self.tool.as_ref()))
            .or_else(|| self.lender.as_deref().and_then(of))
    }

    /// The tool the active tool's armed one-shot pick switches back to after its click
    /// ([`vectorcraft_tools::Tool::returns_to`]).
    pub fn tool_returns_to(&self) -> Option<String> {
        self.tool.returns_to().map(str::to_string)
    }

    /// [`DocState::proxy_mixed`] of the active document for the Appearance panel's active item.
    /// A gradient mesh with points picked ([`Session::fill_mesh_points`]) counts with their colors.
    /// A proxy that stands for the color the active tool paints with ([`Session::tool_paints`]) is
    /// not mixed ([`Session::proxy_source`]).
    pub fn proxy_mixed(&self) -> (bool, bool) {
        if self.tool_paints(false) && self.tool_paints(true) {
            return (false, false);
        }
        let (fill, stroke) = self.active().map_or((false, false), |st| st.proxy_mixed_with(self.appearance_item(), &self.fill_mesh_points()));
        (fill && !self.tool_paints(false), stroke && !self.tool_paints(true))
    }
}

fn proxies(s: &mut Session, _: &Value) -> Result<Value> {
    let (fill, stroke) = s.proxy_paints();
    let (fill_mixed, stroke_mixed) = s.proxy_mixed();
    // As transparency.info answers an opacity: a percentage to two places.
    let percent = |o: Option<f32>| o.map(|o| (f64::from(o) * 10_000.0).round() / 100.0);
    Ok(json!({
        "fill": fill, "stroke": stroke, "fillActive": s.fill_active, "fillMixed": fill_mixed, "strokeMixed": stroke_mixed,
        "fillOpacity": percent(s.proxy_opacity(false)), "strokeOpacity": percent(s.proxy_opacity(true)),
    }))
}

/// `stroke` param, defaulting to the proxy that is in front.
fn stroke_param(s: &Session, p: &Value, cmd: &str) -> Result<bool> {
    switch_or(p, "stroke", !s.fill_active, cmd)
}

/// Error unless `p` gives `keepModel` as true, false or null, or leaves it out ("`keepModel` must
/// be true or false"). [`paint_from`] calls it first, and each command whose params text lists
/// `keepModel` calls it before it changes anything.
pub(crate) fn check_keep_model(p: &Value, cmd: &str) -> Result<()> {
    check_switches(p, &["keepModel"], cmd)
}

/// The model RGB colours given in `p` are stored in: the document's in a CMYK document (`None` in
/// an RGB one, or with `keepModel`).
pub(super) fn new_color_model(s: &Session, p: &Value) -> Option<vectorcraft_color::cms::Model> {
    let mode = s.active()?.doc.color_mode;
    (mode != vectorcraft_doc::ColorMode::Rgb && !bool_or(p, "keepModel", false)).then(|| mode.model())
}

/// `c` in `model` ([`new_color_model`]) when it is an RGB color, converted through the color
/// settings (the working spaces and their intent) as Document Color Mode converts it. Other colors
/// are returned as given.
pub(crate) fn stored_color(model: Option<vectorcraft_color::cms::Model>, c: Color) -> Color {
    match (c, model) {
        (Color::Rgb { .. }, Some(m)) => {
            let cms = vectorcraft_color::cms::active();
            cms.convert(&c, m, cms.settings().intent)
        }
        _ => c,
    }
}

/// A color given in `p` as the document stores it: an RGB color in a CMYK document (without
/// `keepModel`) becomes CMYK through the color settings ([`stored_color`]).
pub(crate) fn new_color(s: &Session, p: &Value, c: Color) -> Color {
    stored_color(new_color_model(s, p), c)
}

/// Parse a paint from params (color / none / swatch / gradient). None = no paint keys given. A
/// color or gradient (its stops and freeform points) given in RGB takes the document's color
/// model ([`new_color_model`]), converted through the color settings. A `none` or `keepModel`
/// other than true, false or null is an error.
pub(crate) fn paint_from(s: &Session, p: &Value) -> Result<Option<Paint>> {
    check_keep_model(p, "paint")?;
    let model = new_color_model(s, p);
    let new_color = |c: Color| stored_color(model, c);
    if switch_or(p, "none", false, "paint")? {
        return Ok(Some(Paint::None));
    }
    if let Some(name) = str_param(p, "swatch") {
        let d = &s.doc()?.doc;
        // A pattern definition works as its swatch even without a swatch entry.
        if d.swatch(name).is_none() && d.pattern(name).is_some() {
            return Ok(Some(vectorcraft_doc::pattern::pattern_paint(name)));
        }
        let sw = d.swatch(name).ok_or_else(|| EngineError::Other(format!("no swatch `{name}`")))?;
        let tint = tint_param(p, "paint")?;
        return match &sw.paint {
            Paint::Solid { .. } => swatch_solid(d, name, tint).map(Some).map_err(|e| bad("paint", e)),
            _ if tint.is_some() => Err(bad("paint", "`tint` applies to global and spot colours")),
            Paint::Gradient(g) => Ok(Some(Paint::Gradient(Box::new(GradientPaint { swatch: Some(name.to_string()), ..(**g).clone() })))),
            other => Ok(Some(other.clone())),
        };
    }
    if let Some(g) = p.get("gradient") {
        check_numbers(g, &["angle"], "paint")?;
        // A stop naming a process swatch keeps the swatch's model.
        let stored = super::gradient::stop_colors(s, p, g);
        let g = super::gradient::link_stops(&s.doc()?.doc, g).map_err(|e| bad("paint", e))?;
        let mut gp = super::gradient::parse_gradient(&super::gradient::with_stored_stops(&g, &Paint::None, &stored)).map_err(|e| bad("paint", e))?;
        if g.get("stops").is_none() {
            // The default stops, white to black.
            gp.gradient.stops.iter_mut().for_each(|st| st.color = new_color(st.color));
        }
        if let Some(f) = &mut gp.freeform {
            f.points.iter_mut().for_each(|pt| pt.color = new_color(pt.color));
        }
        return Ok(Some(Paint::Gradient(Box::new(gp))));
    }
    if let Some(c) = p.get("color") {
        let c = color_value(c).ok_or_else(|| bad("paint", format!("bad color {c}")))?;
        return Ok(Some(Paint::solid(new_color(c))));
    }
    Ok(None)
}

/// The `tint` param (a percentage) as 0..1.
pub(crate) fn tint_param(p: &Value, cmd: &str) -> Result<Option<f32>> {
    p.get("tint")
        .filter(|v| !v.is_null())
        .map(|v| v.as_f64().map(|t| (t / 100.0).clamp(0.0, 1.0) as f32).ok_or_else(|| bad(cmd, "`tint` must be a number (%)")))
        .transpose()
}

/// The solid paint colour swatch `name` applies at `tint` (0..1): a global or spot colour links to
/// itself (default 100%), a tint swatch to its base (default: its own tint), a process colour is
/// just its colour (it has no tints).
pub(crate) fn swatch_solid(d: &Document, name: &str, tint: Option<f32>) -> std::result::Result<Paint, String> {
    let sw = d.swatch(name).ok_or_else(|| format!("no swatch `{name}`"))?;
    if sw.paint.color().is_none() {
        return Err(format!("`{name}` isn't a colour swatch"));
    }
    match (d.swatch_link(name), tint) {
        // A tint swatch whose base is gone keeps its colour.
        (Some((base, own)), _) => Ok(d.tint_paint(&base, tint.unwrap_or(own)).unwrap_or_else(|| sw.paint.clone())),
        (None, None) => Ok(sw.paint.clone()),
        (None, Some(_)) => Err(format!("`{name}` is a process colour: tints apply to global and spot colours")),
    }
}

fn set_paint(s: &mut Session, p: &Value, fill: bool) -> Result<Value> {
    let cmd = if fill { "paint.setFill" } else { "paint.setStroke" };
    if p.get("tint").is_some() && p.get("swatch").is_none() {
        return Err(bad(cmd, "`tint` goes with a global or spot `swatch`"));
    }
    check_keep_model(p, cmd)?;
    let opacity = opacity_param(p, cmd)?;
    let paint = paint_from(s, p)?;
    // A `swatch` that is no name gives no paint, and does not turn the request into an opacity alone.
    let swatch_not_a_name = p.get("swatch").is_some_and(|v| !v.is_null() && !v.is_string());
    if paint.is_none() && (opacity.is_none() || swatch_not_a_name) {
        return Err(bad(cmd, "give color, none, swatch, gradient or opacity"));
    }
    // As `paint.editGradient`: a `dither` given applies to gradient fills only. A gradient swatch
    // (named) keeps its own dither; `"swatch": null` names none.
    if !fill && matches!(&paint, Some(Paint::Gradient(g)) if g.gradient.dither) && p.get("swatch").and_then(Value::as_str).is_none() {
        return Err(bad(cmd, "`dither` applies to gradient fills (paint.setFill)"));
    }
    apply_paint_opacity(s, p, paint, opacity, fill)
}

/// The `opacity` param of `paint.setFill` and `paint.setStroke` (a percentage) as 0..1; `None`
/// when it is left out or null.
fn opacity_param(p: &Value, cmd: &str) -> Result<Option<f32>> {
    match p.get("opacity") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_f64() {
            Some(o) if (0.0..=100.0).contains(&o) => Ok(Some((o / 100.0) as f32)),
            _ => Err(bad(cmd, "`opacity` must be a number from 0 to 100")),
        },
    }
}

/// Whether `p` gives `key` a value other than null.
fn names(p: &Value, key: &str) -> bool {
    p.get(key).is_some_and(|v| !v.is_null())
}

/// The error for an `opacity` that would go to type characters, which have no opacity of their own.
fn characters_have_no_opacity(cmd: &str) -> EngineError {
    bad(cmd, "type characters have no opacity of their own: add a fill or stroke with the Appearance panel's Add New Fill or Add New Stroke")
}

/// [`apply_paint`] on the proxy named by `stroke` (default: the active one), focusing it.
fn apply_to_proxy(s: &mut Session, p: &Value, paint: Paint, cmd: &str) -> Result<Value> {
    let fill = !stroke_param(s, p, cmd)?;
    apply_paint(s, p, paint, fill)
}

/// Set the fill (or stroke) of the targets of `p` (the `item` and `ids` params, see
/// `paint.setFill`) and of the defaults for new art, focus its proxy unless `focus` is false, and
/// remember the paint. Replies `{objects}`: how many objects took the paint.
pub(crate) fn apply_paint(s: &mut Session, p: &Value, paint: Paint, fill: bool) -> Result<Value> {
    apply_paint_opacity(s, p, Some(paint), None, fill)
}

/// [`apply_paint`] with `paint` optional, which also sets the `opacity` (0..1) of the fill (or
/// stroke) item each target takes it on, in the same undo step: the picked gradient mesh points'
/// own opacity for a fill (with no paint or a solid color), nothing on a target without that item.
/// The opacity alone changes neither the defaults for new art nor the remembered colors; with New
/// Art Has Basic Appearance off, new art copies the first selected object's appearance after any
/// command, its opacity included. An opacity that would go only to type
/// characters, to an object selected in a symbol instance, or to no target at all is an error.
fn apply_paint_opacity(s: &mut Session, p: &Value, paint: Option<Paint>, opacity: Option<f32>, fill: bool) -> Result<Value> {
    let cmd = if fill { "paint.setFill" } else { "paint.setStroke" };
    let for_tool = tool_color_params(s, p, cmd, !fill)?;
    let p = for_tool.as_ref().unwrap_or(p);
    // Character edits can return before appearance-target resolution; validate first.
    super::appearance::aligned(p, cmd)?;
    super::appearance::targets_contents(p)?;
    let focus = switch_or(p, "focus", true, cmd)?;
    // An opacity alone places no gradient, so `span` may only be "object" with it.
    let span = spans_targets(p, paint.as_ref().unwrap_or(&Paint::None)).map_err(|e| bad(cmd, e))?;
    // Characters selected with the Type tool take the paint, not the whole type object (#1063). A
    // null `ids` or `id` names no object, as `given_nodes` reads it.
    let on_characters = !names(p, "ids") && !names(p, "id") && p.get("item").is_none();
    // An opacity for the characters is refused before their typing session ends, so the session
    // and its undo step stay as they are.
    if on_characters && opacity.is_some() && super::textedit::selected_range(s).is_some_and(|(_, a, b)| a != b) {
        return Err(characters_have_no_opacity(cmd));
    }
    if on_characters
        && let Some((id, a, b)) = super::textedit::editing_range(s)?
        && a != b
    {
        let (Some(paint), None) = (paint, opacity) else { return Err(characters_have_no_opacity(cmd)) };
        // After Select All in threaded text, every frame's text takes it.
        let story = super::textedit::editing_story(s);
        let ranges: Vec<(NodeId, usize, usize)> =
            if story.is_empty() { vec![(id, a, b)] } else { story.iter().map(|f| (*f, 0, usize::MAX)).collect() };
        let ids: Vec<NodeId> = ranges.iter().map(|(f, _, _)| *f).collect();
        // When the type comes out as it was, the document and its undo history stay as they are.
        let before = s.doc()?.doc.clone();
        // The graph labels whose whole text takes the paint: a graph keeps it for their kind of label.
        let whole: Vec<NodeId> = ranges
            .iter()
            .filter(|(id, a, b)| {
                *a == 0 && matches!(before.node(*id).map(|n| &n.kind), Some(NodeKind::Text(t)) if *b >= vectorcraft_text::edit::runs_len(&t.runs))
            })
            .map(|(id, _, _)| *id)
            .collect();
        super::overprint::edit_counted(s, if fill { "Fill Color" } else { "Stroke Color" }, |d| {
            for &(id, a, b) in &ranges {
                let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) else { return Err(EngineError::NoNode(id)) };
                let (xf, lb) = (t.xf, t.local_bounds());
                vectorcraft_text::edit::style_range(&mut t.runs, a, b, |st| {
                    if fill {
                        st.fill = place_run_paint(&paint, p, xf, lb);
                    } else {
                        if !paint.is_none() {
                            run_stroke_weight(st);
                        }
                        st.stroke = place_run_paint(&paint, p, xf, vectorcraft_doc::appearance::stroke_paint_bounds(lb, st.stroke_width));
                    }
                });
                super::typecmd::refresh_bounds(t);
            }
            let labels = super::graph::capture_label_styles(d, &whole, &|st| {
                if fill {
                    st.fill = unplaced(&paint);
                } else {
                    if !paint.is_none() {
                        run_stroke_weight(st);
                    }
                    st.stroke = unplaced(&paint);
                }
            });
            Ok(super::appearance::count_changed(d, &before, &ids) + usize::from(labels))
        })?;
        if focus {
            s.fill_active = fill;
        }
        s.remember_paint(&paint);
        return Ok(json!({"objects": ids.len()}));
    }
    // An object of a dynamic symbol instance's art selected with Direct Selection: that object's
    // fill or stroke changes for this instance only (solid, none, gradient or pattern).
    if let Some((id, path)) = selected_part_target(s, p, cmd)? {
        let Some(paint) = paint.filter(|_| opacity.is_none()) else {
            return Err(bad(cmd, "an object selected in a symbol instance has no fill or stroke opacity of its own"));
        };
        super::brushsym::edit_part(s, id, &path, if fill { "Fill Color" } else { "Stroke Color" }, |o| {
            if fill {
                o.fill = Some(paint.clone());
            } else {
                o.stroke = Some(paint.clone());
            }
        })?;
        paint_new_art(s, &paint, fill);
        if focus {
            s.fill_active = fill;
        }
        s.remember_paint(&paint);
        return Ok(json!({"objects": 1}));
    }
    // Resolve the requested targets before mutating defaults or the active proxy.
    let item = item_target(s, p, cmd)?.of_kind(s, fill);
    let selected = item.targets_for(s, p, fill)?;
    let ids = super::graph::paint_targets(&s.doc()?.doc, &selected);
    // An opacity has nothing to go to without targets; with a paint, the defaults then stay too.
    if opacity.is_some() && ids.is_empty() {
        return Err(bad(cmd, "select objects or give ids"));
    }
    let of = super::appearance::reference_of(s, p, item, cmd)?;
    let paint = match paint {
        Some(paint) if span => {
            // With a design's art, the gradient spans only the objects with a row equal to the art's, which take it. Each
            // object's box is that of the row it edits.
            let d = &s.doc()?.doc;
            let mut spanned = Vec::with_capacity(ids.len());
            for (id, it) in ids.iter().zip(item.per_target_of(s, of, &ids)) {
                let (Some(it), Some(n)) = (it, d.node(*id)) else { continue };
                spanned.push((*id, it.resolve(&n.appearance, fill, cmd)?));
            }
            Some(placed_across(d, &spanned, paint, p, fill))
        }
        paint => paint,
    };
    // The gradient mesh points picked with the Mesh tool, Direct Selection or the Lasso: a solid
    // fill recolors just those points of their meshes (the proxies show them: `fill_mesh_points`),
    // and a fill opacity fades just them.
    let mesh_points = if fill && matches!(paint, Some(Paint::Solid { .. }) | None) { picked_mesh_points(s, &ids) } else { vec![] };
    // Without `item`, unless the Appearance panel's active fill item takes the paint.
    let colorized = match &paint {
        Some(paint) if fill && p.get("item").is_none() && item == ItemTarget::Top => {
            colorized_images(&s.doc()?.doc, &ids, paint).map_err(|e| bad(cmd, e))?
        }
        _ => vec![],
    };
    let label = match paint {
        None => "Opacity",
        Some(_) if fill => "Fill Color",
        Some(_) => "Stroke Color",
    };
    // Each object whose appearance took the paint, with the row it went to, for the paint a graph stores for its series
    // (a series mark is a path, so it always has a row here).
    let written = std::cell::RefCell::new(Vec::new());
    // The type objects whose characters took the paint (no row targeted): only those change what a graph's labels keep
    // for their characters; a row edit is kept with the label's rows.
    let characters = std::cell::RefCell::new(Vec::new());
    // The targets that took the opacity, and whether one would have given it to its characters.
    let (took_opacity, characters_skipped) = (std::cell::RefCell::new(std::collections::BTreeSet::new()), std::cell::Cell::new(false));
    // The row each target took the opacity on, for what a graph keeps.
    let opacity_written = std::cell::RefCell::new(Vec::new());
    let paint_one = |n: &mut Node, index: Option<usize>, paint: &Paint| -> Result<()> {
        if index.is_none() && matches!(n.kind, NodeKind::Text(_)) {
            characters.borrow_mut().push(n.id);
        }
        if index.is_none() && colorized.contains(&n.id) {
            let colorize = paint.color().map(|_| Box::new(paint.clone()));
            // Without a color the image has nothing to overprint, and a later color starts
            // knocking out.
            if colorize.is_none() {
                n.extras_mut().colorize_overprint = false;
            }
            n.extras_mut().colorize = colorize;
            return Ok(());
        }
        if index.is_none()
            && let Paint::Solid { color, .. } = paint
            && let Some((_, points)) = mesh_points.iter().find(|(id, _)| *id == n.id)
            && let NodeKind::Mesh(m) = &mut n.kind
        {
            for i in points {
                if let Some(q) = m.points.get_mut(*i) {
                    q.color = *color;
                }
            }
            return Ok(());
        }
        if index.is_none()
            && let NodeKind::Text(t) = &mut n.kind
        {
            let (xf, lb) = (t.xf, t.local_bounds());
            for r in &mut t.runs {
                if !fill && !paint.is_none() {
                    run_stroke_weight(&mut r.style);
                }
                let (cur, b) = run_paint_mut(r, !fill, lb);
                *cur = place_run_paint(paint, p, xf, b);
            }
            return Ok(());
        }
        // A missing top fill or stroke is created first: a new stroke's weight sizes the box its
        // gradient fits.
        if n.appearance.paint_at(index, fill).is_none() {
            n.appearance.set_paint_at(index, fill, Paint::None);
        }
        if !fill {
            restore_unpainted_stroke(&mut n.appearance, index, paint);
        }
        let new_paint = keep_pattern_xf(paint, p, n.appearance.paint_at(index, fill));
        let placed = place_paint(&new_paint, p, item_paint_bounds(n, index, fill));
        n.appearance.set_paint_at(index, fill, placed);
        if !fill
            && paint.is_none()
            && let Some(stroke) = n.appearance.stroke_at_mut(index)
        {
            // A stroke without paint has no weight; painting it again restores the 1 pt default.
            stroke.width = 0.0;
        }
        if !fill {
            super::object::snap_if_aligned(n);
        }
        let at = match index {
            Some(_) => n.appearance.item_of_kind(index, fill),
            None => n.appearance.items.iter().rposition(|it| it.is_fill() == fill),
        };
        written.borrow_mut().extend(at.map(|at| (n.id, at)));
        Ok(())
    };
    // The opacity goes to what the paint went to: the picked mesh points, else the fill or stroke
    // item. Characters, and targets without that item, are left as they are.
    let opacity_one = |n: &mut Node, index: Option<usize>, o: f32| {
        if index.is_none() && matches!(n.kind, NodeKind::Text(_)) {
            characters_skipped.set(true);
            return;
        }
        if index.is_none()
            && let Some((_, points)) = mesh_points.iter().find(|(id, _)| *id == n.id)
            && let NodeKind::Mesh(m) = &mut n.kind
        {
            for i in points {
                if let Some(q) = m.points.get_mut(*i) {
                    q.opacity = o;
                    took_opacity.borrow_mut().insert(n.id);
                }
            }
            return;
        }
        let item =
            if fill { n.appearance.fill_at_mut(index).map(|f| &mut f.opacity) } else { n.appearance.stroke_at_mut(index).map(|s| &mut s.opacity) };
        if let Some(item) = item {
            *item = o;
            took_opacity.borrow_mut().insert(n.id);
            let at = index.or_else(|| n.appearance.items.iter().rposition(|it| it.is_fill() == fill));
            opacity_written.borrow_mut().extend(at.map(|at| (n.id, at)));
        }
    };
    // The objects counted: with a design's art, those with a row equal to the art's.
    let ids = edit_items_then(
        s,
        &ids,
        item,
        of,
        cmd,
        label,
        fill,
        |n, index| {
            if let Some(paint) = &paint {
                paint_one(n, index, paint)?;
            }
            if let Some(o) = opacity {
                opacity_one(n, index, o);
            }
            Ok(())
        },
        |before, d, changed| {
            // An opacity that no target took is an error, which also takes back the paint given with it.
            if opacity.is_some() && took_opacity.borrow().is_empty() {
                if characters_skipped.get() {
                    return Err(characters_have_no_opacity(cmd));
                }
                return Err(bad(cmd, "no target has that fill or stroke to take the opacity"));
            }
            // A graph keeps the opacity of a series mark's topmost visible fill and stroke for the whole series, and of a
            // graph line's, also when the object already showed it.
            let kept = opacity.is_some() && {
                let rows = opacity_written.borrow();
                let on = super::graph::RowWrites { opacity: true, ..super::graph::RowWrites::NONE };
                let (on_fill, on_stroke) = if fill { (on, super::graph::RowWrites::NONE) } else { (super::graph::RowWrites::NONE, on) };
                let marks = super::appearance::capture_series_changes(before, d, changed);
                let written = super::appearance::capture_series_opacity_writes(d, &rows);
                marks | written | super::graph::capture_line_row_writes(d, &rows, on_fill, on_stroke)
            };
            let Some(paint) = &paint else { return Ok(kept) };
            let series = super::appearance::capture_series_paint_writes(before, d, changed, &written.borrow());
            // A graph line keeps the paint written to it, also when it already showed it.
            let lines = super::graph::capture_line_paint_writes(d, &written.borrow());
            let labels = super::graph::capture_label_styles(d, &characters.borrow(), &|st| {
                if fill {
                    st.fill = unplaced(paint);
                } else {
                    if !paint.is_none() {
                        run_stroke_weight(st);
                    }
                    st.stroke = unplaced(paint);
                }
            });
            Ok(series || lines || labels || kept)
        },
    )?;
    if focus {
        s.fill_active = fill;
    }
    // Rejected edits leave the next shape's appearance and active proxy untouched.
    if let Some(paint) = &paint {
        paint_new_art(s, paint, fill);
        s.remember_paint(paint);
    }
    // An opacity alone counts only the targets that took it.
    let objects = if paint.is_none() { took_opacity.borrow().len() } else { ids.iter().collect::<std::collections::BTreeSet<_>>().len() };
    Ok(json!({"objects": objects}))
}

/// Does `p` name no `ids`, `id` or `item`?
fn names_no_target(p: &Value) -> bool {
    // A null `ids` or `id` is no target, as `given_nodes` reads it; `item: null` names the top row.
    ["ids", "id"].iter().all(|k| p.get(k).is_none_or(Value::is_null)) && p.get("item").is_none()
}

/// Does the active tool paint with the chosen fill (or stroke, `stroke`: [`Session::tool_paints`],
/// the Live Paint Bucket's fill and stroke, the Shape Builder's fill), or does `p` give
/// `defaultsOnly: true`, while `p` names no `ids`, `id` or `item`? A fill, stroke or stroke weight
/// of that kind chosen then is what the tool paints with, so it goes to the defaults and the
/// selection keeps its paint.
pub(crate) fn for_tool(s: &Session, p: &Value, stroke: bool) -> bool {
    names_no_target(p) && (s.tool_paints(stroke) || asks_defaults_only(p))
}

/// Does `p` give `defaultsOnly: true`?
pub(crate) fn asks_defaults_only(p: &Value) -> bool {
    p.get("defaultsOnly") == Some(&Value::Bool(true))
}

/// `p` with `ids: []` (and without `defaultsOnly`) when [`for_tool`] for the fill (or stroke,
/// `stroke`): the paint goes to the defaults only. None otherwise. A bad `target` or `aligned` is
/// refused first, as it is when the selection takes the paint, and so is a `defaultsOnly` that is
/// not a bool or comes with `ids`, `id` or `item`. When the active tool routed the command, the
/// journal entry notes `defaultsOnly: true`, so replaying it with another tool active changes the
/// defaults only as well.
pub(crate) fn tool_color_params(s: &mut Session, p: &Value, cmd: &str, stroke: bool) -> Result<Option<Value>> {
    let only = switch_or(p, "defaultsOnly", false, cmd)?;
    if only && !names_no_target(p) {
        return Err(bad(cmd, "`defaultsOnly` cannot be combined with `ids`, `id` or `item`"));
    }
    if !for_tool(s, p, stroke) {
        return Ok(None);
    }
    super::appearance::aligned(p, cmd)?;
    super::appearance::targets_contents(p)?;
    if !only {
        s.note_journal("defaultsOnly", json!(true));
    }
    s.note_defaults_only();
    let mut v = p.as_object().cloned().unwrap_or_default();
    v.remove("defaultsOnly");
    v.insert("ids".into(), json!([]));
    Ok(Some(Value::Object(v)))
}

/// The kind of gradient `p` of a gradient command (`cmd`) edits when the active tool can paint
/// with it: the kind `stroke` names, else the proxy in front's when the tool paints with it, else
/// the kind `p` edits on the selection (the Appearance panel's active row's).
pub(crate) fn tool_gradient_kind(s: &Session, p: &Value, cmd: &str) -> Result<bool> {
    check_switches(p, &["stroke"], cmd)?;
    // A tool that paints with both kinds (the Live Paint Bucket), or with neither, needs no kind.
    Ok(if s.tool_paints(false) == s.tool_paints(true) || p.get("stroke").is_some_and(Value::is_boolean) {
        p.get("stroke").and_then(Value::as_bool).unwrap_or(!s.fill_active)
    } else if s.tool_paints(!s.fill_active) {
        !s.fill_active
    } else {
        // A null `ids` or `id` names no target, as `for_tool` reads it.
        let mut q = p.clone();
        if let Some(o) = q.as_object_mut() {
            o.retain(|k, v| !(matches!(k.as_str(), "ids" | "id") && v.is_null()));
        }
        super::appearance::edits_stroke(s, &q, item_target(s, &q, cmd)?, !s.fill_active, cmd)?
    })
}

/// [`tool_color_params`] for `paint.editGradient` or `gradient.presets.apply` (`cmd`), for the kind
/// [`tool_gradient_kind`] gives. The params name that kind as their `stroke` when `p` gives none,
/// since the selection's Appearance row no longer picks it.
pub(crate) fn tool_gradient_params(s: &mut Session, p: &Value, cmd: &str) -> Result<Option<Value>> {
    let stroke = tool_gradient_kind(s, p, cmd)?;
    Ok(tool_color_params(s, p, cmd, stroke)?.map(|mut v| {
        if p.get("stroke").is_none_or(Value::is_null) {
            v["stroke"] = json!(stroke);
        }
        v
    }))
}

/// Whether the paint commands that `cmd` runs with params of its own for the fill (or the stroke,
/// `stroke`) change only the defaults: `p` gives `defaultsOnly: true`, or the active tool paints
/// with that kind ([`Session::tool_paints`]), which the journal entry of `cmd` notes for a replay.
pub(crate) fn defaults_only(s: &mut Session, p: &Value, cmd: &str, stroke: bool) -> Result<bool> {
    if switch_or(p, "defaultsOnly", false, cmd)? {
        s.note_defaults_only();
        return Ok(true);
    }
    let tool = s.tool_paints(stroke);
    if tool {
        s.note_journal("defaultsOnly", json!(true));
        s.note_defaults_only();
    }
    Ok(tool)
}

impl Session {
    /// Note that the running command changed only the paint defaults, with the appearance new art
    /// takes from the selection now ([`Session::inherit_new_art`]). No appearance is captured
    /// while inheritance does not run (New Art Has Basic Appearance on, or the active tool painting
    /// with both the fill and the stroke).
    pub(crate) fn note_defaults_only(&mut self) {
        let inherits = !self.prefs.new_art_basic && !(self.tool_paints(false) && self.tool_paints(true));
        self.defaults_only_ran = Some(if inherits { self.new_art_source() } else { None });
    }
}

/// `q` with `defaultsOnly: true` when `only` ([`defaults_only`]).
pub(crate) fn with_defaults_only(mut q: Value, only: bool) -> Value {
    if only && let Some(o) = q.as_object_mut() {
        o.insert("defaultsOnly".into(), json!(true));
    }
    q
}

/// The Live Paint groups in `roots` (or inside them), given or selected whole, with the ids of
/// their faces and of their edges, found where [`leaf_targets`] finds the objects a paint command
/// changes: a given or selected object other than a group or layer (a blend, say) and a compound
/// shape are painted as one object, and a Live Paint group kept among another one's hidden sources
/// is not one of them. The art of graph designs drawn in graph series is left out.
fn live_paint_groups(d: &Document, roots: &[NodeId]) -> Vec<(NodeId, Vec<NodeId>, Vec<NodeId>)> {
    fn find<'a>(n: &'a Node, top: bool, out: &mut Vec<&'a Node>) {
        let group = matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. });
        if is_live_paint(n) {
            out.push(n);
        } else if (group || !top) && !n.is_compound_shape() && !matches!(n.kind, NodeKind::Compound { .. }) {
            n.children().into_iter().flatten().for_each(|c| find(c, false, out));
        }
    }
    let mut found = vec![];
    for n in roots.iter().filter_map(|r| d.node(*r)) {
        find(n, true, &mut found);
    }
    let mut seen = std::collections::HashSet::new();
    found.retain(|g| seen.insert(g.id));
    // A given or selected compound shape or compound path (or one inside a given or selected object) paints its members,
    // also when one of them is given as well. A Live Paint group given inside one that is not given stays a Live Paint group.
    let owns = |id: &NodeId| d.node(*id).is_some_and(|n| n.is_compound_shape() || matches!(n.kind, NodeKind::Compound { .. }));
    let given: std::collections::HashSet<NodeId> = roots.iter().copied().collect();
    found.retain(|g| {
        !d.ancestry(g.id).is_some_and(|up| {
            let outer = up.iter().position(|a| given.contains(a)).unwrap_or(up.len());
            up.iter().skip(outer).any(|a| *a != g.id && owns(a))
        })
    });
    let kept: std::collections::HashSet<NodeId> = super::appearance::drop_design_art(d, found.iter().map(|g| g.id).collect()).into_iter().collect();
    let ids = |parts: Vec<&std::sync::Arc<Node>>| parts.into_iter().map(|part| part.id).collect();
    found.into_iter().filter(|g| kept.contains(&g.id)).map(|g| (g.id, ids(faces(g)), ids(edges(g)))).collect()
}

/// Every object inside the Live Paint groups `groups` (faces, edges and the hidden source paths).
fn inside_live_paint(d: &Document, groups: &[(NodeId, Vec<NodeId>, Vec<NodeId>)]) -> std::collections::HashSet<NodeId> {
    let mut out = std::collections::HashSet::new();
    for n in groups.iter().filter_map(|(g, _, _)| d.node(*g)) {
        n.walk(&mut |c| {
            if c.id != n.id {
                out.insert(c.id);
            }
        });
    }
    out
}

/// The objects inside the Live Paint groups in `roots` (or inside them) that a fill (`fill`) or a
/// stroke given to the whole group leaves as they are: a fill paints the group's faces only, and a
/// stroke its edges only. Their edges (faces) and hidden source paths are left out.
pub(crate) fn live_paint_parts_skipped(d: &Document, roots: &[NodeId], fill: bool) -> std::collections::HashSet<NodeId> {
    let groups = live_paint_groups(d, roots);
    let mut out = inside_live_paint(d, &groups);
    for (_, faces, edges) in &groups {
        for id in if fill { faces } else { edges } {
            out.remove(id);
        }
    }
    out
}

/// Faces, edges and the other objects (the hidden source paths) of Live Paint groups.
pub(crate) type LivePaintParts = (std::collections::HashSet<NodeId>, std::collections::HashSet<NodeId>, std::collections::HashSet<NodeId>);

/// The faces, the edges and the hidden source paths of the Live Paint groups in `roots` (or inside them),
/// found as [`live_paint_parts_skipped`] finds them.
pub(crate) fn live_paint_split(d: &Document, roots: &[NodeId]) -> LivePaintParts {
    let groups = live_paint_groups(d, roots);
    let mut hidden = inside_live_paint(d, &groups);
    let (mut faces, mut edges) = (std::collections::HashSet::new(), std::collections::HashSet::new());
    for (_, f, e) in &groups {
        faces.extend(f.iter().copied());
        edges.extend(e.iter().copied());
    }
    hidden.retain(|id| !faces.contains(id) && !edges.contains(id));
    (faces, edges, hidden)
}

/// The faces and edges in `selected` whose Live Paint group is selected too, itself or inside another selected
/// object: they count with their group, wherever they are in `selected`. The groups are looked for only when an
/// object in `selected` is inside a Live Paint group.
pub(crate) fn live_paint_covered(d: &Document, selected: &[NodeId]) -> std::collections::HashSet<NodeId> {
    if !selected.iter().any(|id| in_live_paint(d, *id)) {
        return std::collections::HashSet::new();
    }
    let (mut parts, edges, _) = live_paint_split(d, selected);
    parts.extend(edges);
    parts
}

/// Is `id` inside a Live Paint group (one of the groups around it is one)?
fn in_live_paint(d: &Document, id: NodeId) -> bool {
    d.ancestry(id).is_some_and(|up| up.iter().rev().skip(1).filter_map(|a| d.node(*a)).any(is_live_paint))
}

impl DocState {
    /// [`live_paint_covered`] for `ids`, the selected objects or the selection's subjects: none
    /// when the [`SelectionPaints`] of the selection say no selected object is in a Live Paint group.
    pub(crate) fn selection_live_paint_covered(&self, ids: &[NodeId]) -> std::collections::HashSet<NodeId> {
        if self.selection_paints().is_some_and(|p| !p.in_live_paint()) {
            return std::collections::HashSet::new();
        }
        live_paint_covered(&self.doc, ids)
    }
}

/// The instance whose object is selected with Direct Selection, and that object's path
/// ([`super::brushsym::selected_part`]), when `p` names no `ids`, `id` or `item`: the paint commands
/// then change that object. A bad `target` or `aligned` is refused as it is for any other selection.
fn selected_part_target(s: &Session, p: &Value, cmd: &str) -> Result<Option<(NodeId, Vec<u32>)>> {
    if names(p, "ids") || names(p, "id") || p.get("item").is_some() {
        return Ok(None);
    }
    let part = super::brushsym::selected_part(s)?;
    if part.is_some() {
        // Read `target` first: `item_target` stops at a null `id` or `ids` before it reads `target`.
        super::appearance::targets_contents(p)?;
        item_target(s, p, cmd)?;
    }
    Ok(part)
}

/// Can Invert / Complement recolour `paint` on an object of a dynamic symbol instance's art selected
/// with Direct Selection? A solid color or none can; a gradient or pattern cannot (one override
/// holds one paint, and those commands recolour solid colours only).
pub(crate) fn part_takes(paint: &Paint) -> bool {
    matches!(paint, Paint::Solid { .. } | Paint::None)
}

/// Make `paint` the fill or stroke of new art. New art takes an unplaced gradient so that it fits
/// the new object's own bounds, and a painted stroke on art that had none gets the 1 pt default.
fn paint_new_art(s: &mut Session, paint: &Paint, fill: bool) {
    if fill {
        s.paint.fill = unplaced(paint);
    } else {
        if !paint.is_none() && s.paint.stroke.is_none() && s.paint.stroke_width == 0.0 {
            s.paint.stroke_width = 1.0;
        }
        s.paint.stroke = unplaced(paint);
    }
}

/// Does `p` ask for `span: "selection"` (#2127)? Only a linear, radial, conic or diamond gradient the targets
/// would each fit to their own bounds can span them: one with its own vector (`start`/`end`),
/// a freeform one or a gradient swatch already has its placement.
fn spans_targets(p: &Value, paint: &Paint) -> std::result::Result<bool, String> {
    match p.get("span") {
        None | Some(Value::Null) => return Ok(false),
        Some(Value::String(v)) if v == "object" => return Ok(false),
        Some(Value::String(v)) if v == "selection" => {}
        Some(v) => return Err(format!("span must be \"object\" or \"selection\", got {v}")),
    }
    let Paint::Gradient(g) = paint else { return Err("span: \"selection\" places a gradient: give `gradient`".into()) };
    if p.get("swatch").is_some() {
        return Err("span: \"selection\" places a `gradient`, not a swatch".into());
    }
    if g.gradient.kind == vectorcraft_color::GradientKind::Freeform || g.freeform.is_some() {
        return Err("span: \"selection\" places a linear, radial, conic or diamond gradient, not a freeform one".into());
    }
    if g.geom.is_some() {
        return Err(
            "span: \"selection\" places the gradient itself: leave out `start` and `end` (they already place it in document coordinates)".into()
        );
    }
    Ok(true)
}

/// `paint` placed once on the union of the bounds of `ids` (each one's fill box, or the box the
/// stroke it edits paints: the row given with it, `None` for the topmost), at the gradient's angle
/// and aspect: every target gets the same vector in document coordinates, so the ramp runs on
/// across them (#2127).
fn placed_across(d: &Document, ids: &[(NodeId, Option<usize>)], paint: Paint, p: &Value, fill: bool) -> Paint {
    let union = ids
        .iter()
        .filter_map(|(id, index)| Some((d.node(*id)?, *index)))
        .filter_map(|(n, index)| item_paint_bounds(n, index, fill).or_else(|| n.geometric_bounds()))
        .filter(|b| [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite()))
        .reduce(|a, b| a.union(b));
    let (Some(union), Paint::Gradient(g)) = (union, &paint) else { return paint };
    // An `aspect` places a radial gradient on the box already; otherwise fit it there.
    match place_paint(&paint, p, Some(union)) {
        Paint::Gradient(placed) if placed.geom.is_some() => Paint::Gradient(placed),
        _ => {
            let geom = vectorcraft_color::GradientGeom::fit(g.gradient.kind, union, g.angle);
            Paint::Gradient(Box::new(GradientPaint { geom: Some(geom), ..(**g).clone() }))
        }
    }
}

/// The images among `ids` that a fill of `paint` colorizes: grayscale and 1-bit images, for a solid
/// color (or none on a colorized image, which takes the color off). The color may be black, white,
/// a process color or a spot color; on an image with an alpha channel, only a spot color, black or
/// white, and other colors are an error. A colorized image that shows a preview that isn't
/// grayscale, such as a missing link's placeholder, takes a solid color as its new colorize paint;
/// while its paint is one an alpha channel allows, a process color is an error there too.
fn colorized_images(d: &Document, ids: &[NodeId], paint: &Paint) -> std::result::Result<Vec<NodeId>, String> {
    let mut out = vec![];
    for id in ids {
        let Some(n) = d.node(*id) else { continue };
        let NodeKind::Image(im) = &n.kind else { continue };
        // `none` takes a color off whatever the image's pixels have become since.
        if matches!(paint, Paint::None) && n.extras().colorize.is_some() {
            out.push(*id);
            continue;
        }
        // The Fill proxy shows such an image's colorize paint. Its pixels are not known until the
        // file is relinked: a process color replaces only a process color, which the image's
        // pixels already accepted, so a relink can't bring back an alpha channel under one.
        if matches!(paint, Paint::Solid { .. })
            && let Some(old) = n.extras().colorize.as_deref()
            && d.image_blob(im).is_some_and(|b| !b.takes_colorize())
        {
            if !colorizes_transparency(d, paint) && colorizes_transparency(d, old) {
                return Err(PROCESS_ON_TRANSPARENCY.into());
            }
            out.push(*id);
            continue;
        }
        // A color-masked image is colorized as it shows: its masked pixels, which have an alpha
        // channel.
        let Some(gray) = d.image_blob(&d.shown_image(n, im)).and_then(|b| b.gray_source()) else { continue };
        match paint {
            // Without a color, `none` goes to the image's fill item.
            Paint::None => continue,
            Paint::Solid { .. } => {
                if gray.alpha && !colorizes_transparency(d, paint) {
                    return Err(PROCESS_ON_TRANSPARENCY.into());
                }
            }
            _ => continue,
        }
        out.push(*id);
    }
    Ok(out)
}

/// The message for a process color on a grayscale image with an alpha channel.
pub(super) const PROCESS_ON_TRANSPARENCY: &str =
    "a process color can't colorize a grayscale image with transparency: use a spot color, black or white";

/// Can `paint` colorize a grayscale image with an alpha channel: a color linked to a spot swatch
/// of `d`, black or white?
fn colorizes_transparency(d: &Document, paint: &Paint) -> bool {
    match paint {
        Paint::Solid { color, swatch, .. } => swatch.as_deref().and_then(|name| d.swatch(name)).is_some_and(|w| w.spot) || black_or_white(color),
        _ => true,
    }
}

/// An error when an image with an alpha channel in `d` (its art, opacity masks, symbols, graph
/// designs and pattern tiles) is colorized with a color that [`colorizes_transparency`] refuses,
/// as a swatch edit that turns a spot color into a process color or recolors a process swatch
/// can leave one.
pub(super) fn check_colorized(d: &Document) -> std::result::Result<(), String> {
    let mut refused = false;
    let mut visit = |n: &Node| {
        vectorcraft_doc::swatches::walk_with_masks(n, &mut |m| {
            if !refused && gray_colorized_with_process(d, m).is_some_and(|g| g.alpha) {
                refused = true;
            }
        })
    };
    d.layers
        .iter()
        .chain(d.symbols.iter().map(|s| &s.art))
        .chain(d.graph_designs.iter().map(|g| &g.art))
        .chain(d.patterns.iter().flat_map(|p| &p.art))
        .for_each(|n| visit(n));
    if refused { Err(PROCESS_ON_TRANSPARENCY.into()) } else { Ok(()) }
}

/// [`check_colorized`] after an edit, when the document passed it before the edit (`was_ok`), so
/// an edit doesn't fail over an image it didn't change.
pub(super) fn check_colorized_kept(was_ok: bool, d: &Document) -> std::result::Result<(), String> {
    if was_ok { check_colorized(d) } else { Ok(()) }
}

/// Is `n` an image colorized with a color that [`colorizes_transparency`] refuses? When its pixels
/// are grayscale or 1-bit, they can't have an alpha channel.
pub(super) fn colorized_with_process(d: &Document, n: &Node) -> bool {
    matches!(n.kind, NodeKind::Image(_)) && n.extras().colorize.as_deref().is_some_and(|paint| !colorizes_transparency(d, paint))
}

/// How the pixels `n` shows are gray, when `n` is a grayscale or 1-bit image colorized with a
/// color that [`colorizes_transparency`] refuses ([`colorized_with_process`]).
pub(super) fn gray_colorized_with_process(d: &Document, n: &Node) -> Option<GraySource> {
    let NodeKind::Image(im) = &n.kind else { return None };
    if colorized_with_process(d, n) { d.image_blob(&d.shown_image(n, im)).and_then(|b| b.gray_source()) } else { None }
}

/// Is `e` the refusal of [`check_colorized`]?
pub(super) fn is_colorize_refusal(e: &EngineError) -> bool {
    e.to_string().contains(PROCESS_ON_TRANSPARENCY)
}

/// Is `c` black or white (white, or black alone at 100%)?
fn black_or_white(c: &Color) -> bool {
    let (none, full) = (|v: f32| v <= 0.0, |v: f32| v >= 1.0);
    match *c {
        Color::Gray { k } => none(k) || full(k),
        Color::Cmyk { c, m, y, k } => none(c) && none(m) && none(y) && (none(k) || full(k)),
        Color::Rgb { r, g, b } => [r, g, b].into_iter().all(none) || [r, g, b].into_iter().all(full),
        Color::Lab { .. } => false,
    }
}

/// The points the active tool has picked (its `meshPoints` option: the Mesh tool's, Direct
/// Selection's or the Lasso's) on the gradient meshes among `ids`, in range, each mesh with its points.
fn picked_mesh_points(s: &Session, ids: &[NodeId]) -> Vec<(NodeId, Vec<usize>)> {
    let o = s.tool_options();
    let (Some(picked), Ok(st)) = (o.get("meshPoints").and_then(Value::as_array), s.doc()) else { return vec![] };
    let mut out: Vec<(NodeId, Vec<usize>)> = vec![];
    for pt in picked {
        let (Some(id), Some(index)) = (pt.get("id").and_then(Value::as_u64).map(NodeId), pt.get("index").and_then(Value::as_u64)) else { continue };
        let Ok(index) = usize::try_from(index) else { continue };
        if !ids.contains(&id) || !matches!(st.doc.node(id).map(|n| &n.kind), Some(NodeKind::Mesh(m)) if index < m.points.len()) {
            continue;
        }
        match out.iter_mut().find(|(m, _)| *m == id) {
            Some((_, v)) => v.push(index),
            None => out.push((id, vec![index])),
        }
    }
    out
}

/// The picked gradient mesh points (see [`picked_mesh_points`]) when each of `ids` is a gradient
/// mesh with points picked: the Transparency panel's opacity is theirs then.
pub(crate) fn picked_mesh_points_of(s: &Session, ids: &[NodeId]) -> Option<Vec<(NodeId, Vec<usize>)>> {
    let v = picked_mesh_points(s, ids);
    (!ids.is_empty() && ids.iter().all(|id| v.iter().any(|(m, _)| m == id))).then_some(v)
}

/// The colors of the picked points of `n` (see [`picked_mesh_points`]) when it is a gradient mesh
/// among `points`, in the order they were picked; empty otherwise.
fn picked_colors(n: &Node, points: &[(NodeId, Vec<usize>)]) -> Vec<Color> {
    match (&n.kind, points.iter().find(|(id, _)| *id == n.id)) {
        (NodeKind::Mesh(m), Some((_, v))) => v.iter().filter_map(|i| m.points.get(*i)).map(|q| q.color).collect(),
        _ => vec![],
    }
}

impl Session {
    /// The picked gradient mesh points that a solid fill color (`paint.setFill` without `ids` or
    /// `item`) recolors instead of their meshes' fills, each mesh with its points: the targets are
    /// resolved as [`set_paint`] resolves them, so this is empty while the Appearance panel's
    /// active item is a fill row or an object of a symbol instance is selected, and a mesh with
    /// points picked counts even when other objects are selected with it.
    pub fn fill_mesh_points(&self) -> Vec<(NodeId, Vec<usize>)> {
        let Some(st) = self.active() else { return vec![] };
        // Only gradient meshes have points to pick: a selection with no mesh and no container that
        // could hold one skips reading the tool's options.
        let may_hold_mesh = |id: &NodeId| st.doc.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Mesh(_)) || n.is_container());
        let none_may = || st.selection_paints().map_or_else(|| !st.selection.subjects().iter().any(may_hold_mesh), |p| !p.may_hold_mesh());
        if st.selection.symbol_part.is_some() || none_may() {
            return vec![];
        }
        let Ok(item) = item_target(self, &Value::Null, "paint.setFill").map(|t| t.of_kind(self, true)) else { return vec![] };
        if item != ItemTarget::Top {
            return vec![];
        }
        let Ok(selected) = item.targets(self, &Value::Null) else { return vec![] };
        picked_mesh_points(self, &super::graph::paint_targets(&st.doc, &selected))
    }

    /// The picked gradient mesh points that the Transparency panel's opacity (`transparency.set`
    /// without `ids` or `item`) fades instead of their objects: `None` unless every target is a
    /// gradient mesh with points picked, and while the Appearance panel's active item takes the
    /// opacity (outside opacity mask editing).
    pub fn mesh_points_picked(&self) -> Option<Vec<(NodeId, Vec<usize>)>> {
        let st = self.active()?;
        let ids = super::opacitymask::transparency_targets(self, &Value::Null).ok()?;
        // Only gradient meshes have points to pick: other targets skip reading the tool's options.
        if ids.is_empty() || !ids.iter().all(|id| matches!(st.doc.node(*id).map(|n| &n.kind), Some(NodeKind::Mesh(_)))) {
            return None;
        }
        if st.doc.mask_edit.is_none() && self.appearance_item().is_some() {
            return None;
        }
        picked_mesh_points_of(self, &ids)
    }
}

/// A type run's stroke; a painted stroke on a run without one gets 1 pt.
pub(super) fn set_run_stroke(style: &mut CharStyle, paint: Paint) {
    if !paint.is_none() {
        run_stroke_weight(style);
    }
    style.stroke = paint;
}

/// `paint` as it replaces `old`: a pattern swatch picked over a pattern keeps the placement that
/// pattern already had, so only the pattern changes (#1970).
pub(super) fn keep_pattern_xf(paint: &Paint, p: &Value, old: Option<&Paint>) -> Paint {
    match (paint, old) {
        (Paint::Pattern { pattern, xf }, Some(Paint::Pattern { xf: kept, .. })) if p.get("swatch").is_some() && *xf == Affine::IDENTITY => {
            Paint::Pattern { pattern: pattern.clone(), xf: *kept }
        }
        _ => paint.clone(),
    }
}

/// Use the existing 1 pt default when adding paint to an unpainted zero-width stroke.
pub(super) fn restore_unpainted_stroke(ap: &mut Appearance, index: Option<usize>, paint: &Paint) {
    if !paint.is_none()
        && let Some(stroke) = ap.stroke_at_mut(index)
        && stroke.paint.is_none()
        && stroke.width == 0.0
    {
        stroke.width = 1.0;
    }
}

/// The paint every one of `parts` has on top as its fill (`fill`) or stroke, swatch link included
/// and gradient placement left out ([`unplaced`]: each part fits it to its own bounds), or None
/// when they differ or there are none. A freeform gradient's points and their colors belong to the
/// part they sit on, so a freeform gradient on any part counts as a paint the parts do not share.
fn shared_paint(d: &Document, parts: &[NodeId], fill: bool) -> Option<Paint> {
    let mut paints = parts.iter().filter_map(|id| d.node(*id)).map(|n| if fill { n.appearance.fill_paint() } else { n.appearance.stroke_paint() });
    let first = paints.next()?;
    let freeform =
        |p: &Paint| matches!(p, Paint::Gradient(g) if g.gradient.kind == vectorcraft_color::GradientKind::Freeform || g.freeform.is_some());
    if freeform(&first) {
        return None;
    }
    let first = unplaced(&first);
    paints.all(|p| !freeform(&p) && unplaced(&p) == first).then_some(first)
}

fn swap(s: &mut Session, p: &Value) -> Result<Value> {
    // The swap changes a fill, so it goes to the defaults only while the active tool paints with the chosen fill.
    let for_tool = tool_color_params(s, p, "paint.swap", false)?;
    let p = for_tool.as_ref().unwrap_or(p);
    // A tool that paints with the fill only (the Shape Builder): the Stroke proxy shows the
    // selection's stroke, which the current fill takes unless the proxy shows mixed strokes. With
    // nothing selected it shows the stroke for new art, which the swap moves as it is.
    // A swap with `defaultsOnly` and no such tool swaps the defaults alone.
    let fill_only = s.tool_paints(false) && !s.tool_paints(true);
    let shown_stroke = (for_tool.is_some() && fill_only && !s.proxy_mixed().1)
        .then(|| s.proxy_source(true).map(|(n, item)| super::gradient::unplaced(&proxy_paint(&n, true, item))))
        .flatten();
    let targets = paint_targets(s, p)?;
    let given = !targets.is_empty();
    // A whole Live Paint group: its faces take the stroke its edges share and its edges the fill its faces share.
    let groups = live_paint_groups(&s.doc()?.doc, &super::targets(s, p)?);
    let inside = inside_live_paint(&s.doc()?.doc, &groups);
    let ids = super::appearance::drop_design_art(&s.doc()?.doc, targets.into_iter().filter(|id| !inside.contains(id)).collect());
    // Only design art was given or selected: nothing changes, the defaults included.
    if given && ids.is_empty() && groups.is_empty() {
        return ok();
    }
    if !ids.is_empty() || !groups.is_empty() {
        // When every object comes out as it was, the document and its undo history stay as they are.
        let before = s.doc()?.doc.clone();
        // A group whose faces' fills (or edges' strokes) differ or are freeform gradients, or that has no face or no edge,
        // keeps its paint.
        let swaps: Vec<(&[NodeId], &[NodeId], Paint, Paint)> = groups
            .iter()
            .filter_map(|(_, faces, edges)| {
                Some((faces.as_slice(), edges.as_slice(), shared_paint(&before, faces, true)?, shared_paint(&before, edges, false)?))
            })
            .collect();
        let parts: Vec<NodeId> = groups.iter().flat_map(|(_, faces, edges)| faces.iter().chain(edges)).copied().collect();
        super::overprint::edit_counted(s, "Swap Fill and Stroke", |d| {
            for (faces, edges, fill, stroke) in &swaps {
                for id in *faces {
                    if let Some(n) = d.node_mut(*id) {
                        n.appearance.set_fill(stroke.clone());
                    }
                }
                for id in *edges {
                    if let Some(n) = d.node_mut(*id) {
                        restore_unpainted_stroke(&mut n.appearance, None, fill);
                        n.appearance.set_stroke(fill.clone());
                    }
                }
            }
            let parts_changed = super::appearance::count_changed(d, &before, &parts);
            for id in &ids {
                let Some(n) = d.node_mut(*id) else { continue };
                if let NodeKind::Text(t) = &mut n.kind {
                    for r in &mut t.runs {
                        let fill = std::mem::take(&mut r.style.fill);
                        r.style.fill = std::mem::take(&mut r.style.stroke);
                        set_run_stroke(&mut r.style, fill);
                    }
                    continue;
                }
                let f = n.appearance.fill_paint();
                let st = n.appearance.stroke_paint();
                n.appearance.set_fill(st);
                restore_unpainted_stroke(&mut n.appearance, None, &f);
                n.appearance.set_stroke(f);
            }
            // The swap writes the paint of the topmost fill and stroke, not their opacity.
            let kept = super::appearance::capture_series_writes(&before, d, &ids, (true, true), (false, false), true, false);
            let labels = super::graph::capture_label_changes(&before, d, &ids, false, None);
            Ok(super::appearance::count_changed(d, &before, &ids) + parts_changed + usize::from(kept || labels))
        })?;
    }
    if s.paint.stroke.is_none() && !s.paint.fill.is_none() && s.paint.stroke_width == 0.0 {
        s.paint.stroke_width = 1.0;
    }
    std::mem::swap(&mut s.paint.fill, &mut s.paint.stroke);
    if let Some(stroke) = shown_stroke {
        s.paint.fill = stroke;
    }
    ok()
}

/// The weight of the stroke [`paint.default`](default_paint) gives the edges of a whole Live Paint group.
const LIVE_PAINT_DEFAULT_WEIGHT: f64 = 0.5;

fn default_paint(s: &mut Session, p: &Value) -> Result<Value> {
    // As with paint.swap, the new fill decides whether only the defaults change.
    let for_tool = tool_color_params(s, p, "paint.default", false)?;
    let p = for_tool.as_ref().unwrap_or(p);
    let targets = paint_targets(s, p)?;
    let given = !targets.is_empty();
    // A whole Live Paint group: its faces lose their fill and its edges take a black 0.5 pt stroke.
    let groups = live_paint_groups(&s.doc()?.doc, &super::targets(s, p)?);
    let inside = inside_live_paint(&s.doc()?.doc, &groups);
    let ids = super::appearance::drop_design_art(&s.doc()?.doc, targets.into_iter().filter(|id| !inside.contains(id)).collect());
    // Only design art was given or selected: nothing changes, the defaults included.
    if given && ids.is_empty() && groups.is_empty() {
        return ok();
    }
    if !ids.is_empty() || !groups.is_empty() {
        // When every object comes out as it was, the document and its undo history stay as they are.
        let before = s.doc()?.doc.clone();
        // A redefined Default Graphic Style gives other art its fills, strokes and effects, placed on each object.
        let style = super::newart::redefined_default_style(&before).cloned();
        let parts: Vec<NodeId> = groups.iter().flat_map(|(_, faces, edges)| faces.iter().chain(edges)).copied().collect();
        super::overprint::edit_counted(s, "Default Fill and Stroke", |d| {
            for (_, faces, edges) in &groups {
                let face = Appearance { items: vec![AppearanceItem::Fill(FillLayer::new(Paint::None))], ..Appearance::default() };
                let edge = Appearance {
                    items: vec![AppearanceItem::Stroke(StrokeLayer::new(Paint::solid(Color::BLACK), LIVE_PAINT_DEFAULT_WEIGHT))],
                    ..Appearance::default()
                };
                for (ids, look) in [(faces, &face), (edges, &edge)] {
                    for id in ids {
                        if let Some(n) = d.node_mut(*id) {
                            n.appearance = look.clone().into();
                        }
                    }
                }
            }
            let parts_changed = super::appearance::count_changed(d, &before, &parts);
            for id in &ids {
                let Some(n) = d.node_mut(*id) else { continue };
                match &mut n.kind {
                    NodeKind::Text(t) => {
                        for r in &mut t.runs {
                            r.style.fill = Paint::solid(Color::BLACK);
                            r.style.stroke = Paint::None;
                        }
                    }
                    _ => {
                        n.appearance = match &style {
                            Some(g) => g.appearance_on(n).into_owned().into(),
                            None => Appearance::default_art().into(),
                        }
                    }
                }
            }
            let kept = super::appearance::capture_series_writes(&before, d, &ids, (true, true), (true, true), false, false);
            // The graph keeps the default paint for every label of a kind it reached, and the default fill and stroke rows
            // for every line of a kind it reached, also when one already showed them; the object opacity a line kind keeps
            // stays.
            let labels = super::graph::capture_label_styles(d, &ids, &|st| {
                st.fill = Paint::solid(Color::BLACK);
                st.stroke = Paint::None;
            });
            let lines = super::graph::capture_line_fill_stroke_rows(d, &ids);
            Ok(super::appearance::count_changed(d, &before, &ids) + parts_changed + usize::from(kept || labels || lines))
        })?;
    }
    if ids.is_empty() && !groups.is_empty() {
        // With only whole Live Paint groups given or selected, new art takes what their parts took, and nothing else.
        s.paint = vectorcraft_tools::PaintDefaults {
            fill: Paint::None,
            stroke: Paint::solid(Color::BLACK),
            stroke_width: LIVE_PAINT_DEFAULT_WEIGHT,
            ..Default::default()
        };
    } else {
        super::newart::reset(s);
    }
    ok()
}

/// Invert / Complement (`cmd`, undone as `label`): recolour the active proxy of the targets (or the
/// default when there are none) and remember the first target's new paint. The Appearance panel's
/// active item of the proxy's kind is recoloured in each selected object's own stack instead.
fn proxy_recolor(s: &mut Session, p: &Value, cmd: &str, label: &str, f: &dyn Fn(Color) -> Color) -> Result<Value> {
    let stroke = stroke_param(s, p, cmd)?;
    let for_tool = tool_color_params(s, p, cmd, stroke)?;
    let p = for_tool.as_ref().unwrap_or(p);
    // An object of a dynamic symbol instance's art selected with Direct Selection: the solid color
    // it shows is recolored for this instance only. A gradient or pattern is refused, and so is type
    // whose characters differ in that paint, since one change holds one paint.
    if let Some((id, path)) = selected_part_target(s, p, cmd)? {
        let part = s.doc()?.selected_symbol_part();
        let mut paint = part.as_ref().map_or(Paint::None, |n| proxy_paint(n, stroke, None));
        let runs_differ = match part.as_ref().map(|n| &n.kind) {
            Some(NodeKind::Text(t)) => t.runs.iter().any(|r| !same_in_proxy(if stroke { &r.style.stroke } else { &r.style.fill }, &paint)),
            _ => false,
        };
        if runs_differ || !part_takes(&paint) {
            return Err(super::brushsym::part_only());
        }
        let changed = super::colorcmds::map_paint(&mut paint, f);
        if changed {
            super::brushsym::edit_part(s, id, &path, label, |o| {
                if stroke {
                    o.stroke = Some(paint.clone());
                } else {
                    o.fill = Some(paint.clone());
                }
            })?;
            s.remember_paint(&paint);
        }
        return Ok(json!({ "changed": changed as usize }));
    }
    let ids = match super::given_objects(s, p, cmd)? {
        Some(ids) => ids,
        None => super::edit::selected_roots(s)?,
    };
    if ids.is_empty() {
        let def = if stroke { &mut s.paint.stroke } else { &mut s.paint.fill };
        let changed = super::colorcmds::map_paint(def, f);
        let shown = def.clone();
        s.remember_paint(&shown);
        return Ok(json!({ "changed": changed as usize }));
    }
    let item = item_target(s, p, cmd)?.of_kind(s, !stroke);
    // Read before the check below, so that a bad param (such as `target` or `aligned`, or an explicit `item` the art has
    // no row for) is still an error.
    let targets = super::appearance::drop_design_art_rows(s, item.targets_for(s, p, !stroke)?, item, !stroke, cmd)?;
    let of = match item {
        ItemTarget::Item { .. } => super::appearance::reference_of(s, p, item, cmd)?,
        ItemTarget::Top => None,
    };
    // Read before the check below too: `targets` reads no `target` for the topmost fill or stroke.
    let contents = super::appearance::targets_contents(p)?;
    let ids = super::appearance::drop_design_art(&s.doc()?.doc, ids);
    // Only design art was given or selected: nothing changes, the defaults and remembered colors included.
    if ids.is_empty() {
        return Ok(json!({ "changed": 0 }));
    }
    let (r, shown_at) = match item {
        ItemTarget::Item { .. } => {
            // Only design art was given or selected: nothing changes, the remembered colors included.
            if targets.is_empty() {
                return Ok(json!({ "changed": 0 }));
            }
            // The row shown is the one edited in the first object that has it: an aligned row maps to each object's equal row.
            let shown_at = super::appearance::first_edited(s, item, of, &targets).and_then(|(id, it)| match it {
                ItemTarget::Item { index, .. } => Some((id, Some(index))),
                ItemTarget::Top => None,
            });
            let mut changed = 0;
            edit_items(s, &targets, item, of, cmd, label, !stroke, |n, i| {
                let paint =
                    if stroke { n.appearance.stroke_at_mut(i).map(|l| &mut l.paint) } else { n.appearance.fill_at_mut(i).map(|l| &mut l.paint) };
                changed += paint.is_some_and(|paint| super::colorcmds::map_paint(paint, f)) as usize;
                Ok(())
            })?;
            (json!({ "changed": changed }), shown_at)
        }
        // The proxy recolors its paints, including a colorized image's fill, without changing image pixels or pattern tiles.
        // A given group's own fills and strokes change with the objects inside it, except with `target: "contents"`; other
        // objects change as their paint targets do (a compound path's member stands for the compound path). Design art
        // inside a given graph or series group is left out.
        ItemTarget::Top => {
            let mut roots = vec![];
            for id in &ids {
                let n = s.doc()?.doc.node(*id);
                let group =
                    !contents && n.is_some_and(|n| matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. }) && !n.is_compound_shape());
                if group {
                    roots.push(*id);
                } else {
                    roots.extend(leaf_targets(s, &[*id])?);
                }
            }
            let d = &s.doc()?.doc;
            let mut seen = std::collections::HashSet::new();
            roots.retain(|id| seen.insert(*id));
            let roots = super::appearance::drop_design_art(d, roots);
            if roots.is_empty() {
                return Ok(json!({ "changed": 0 }));
            }
            // A Live Paint group's fill is its faces' fills and its stroke its edges' strokes, also with `target: "contents"`;
            // like any group, it recolors rows of its own as well.
            let parts_skipped = live_paint_parts_skipped(d, &ids, !stroke);
            let roots: Vec<NodeId> = roots.into_iter().filter(|id| !parts_skipped.contains(id)).collect();
            if roots.is_empty() {
                return Ok(json!({ "changed": 0 }));
            }
            let mut art: std::collections::HashSet<NodeId> = super::appearance::design_art_in(d, &roots).into_iter().collect();
            art.extend(parts_skipped.iter().copied());
            // The fill or stroke the proxy shows of each group among the roots, before the edit (a Live Paint group shows
            // its faces' or edges').
            let own = |d: &Document, id: NodeId| {
                d.node(id)
                    .filter(|n| matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. }) && !is_live_paint(n))
                    .and_then(|n| n.proxy_paint_only(stroke, None))
                    .cloned()
            };
            let own_before: std::collections::HashMap<NodeId, Paint> = roots.iter().filter_map(|id| Some((*id, own(d, *id)?))).collect();
            let q = json!({"fill": !stroke, "stroke": stroke, "includeImages": false, "includePatterns": false, "ids": roots.iter().map(|i| i.0).collect::<Vec<_>>()});
            let r = super::colorcmds::recolor_except(s, &q, cmd, label, f, art)?;
            // As the proxy shows a group: its own fill or stroke when the edit changed it, else the first object inside it
            // that is not design art (which keeps its paint). The first object given that has either shows it.
            let d = &s.doc()?.doc;
            let mut shown_at = None;
            for id in &ids {
                if let Some(was) = own_before.get(id)
                    && own(d, *id).as_ref() != Some(was)
                {
                    shown_at = Some((*id, None));
                    break;
                }
                let leaves = leaf_targets(s, &[*id])?.into_iter().filter(|leaf| !parts_skipped.contains(leaf)).collect();
                if let Some(leaf) = super::appearance::drop_design_art(d, leaves).first() {
                    shown_at = Some((*leaf, None));
                    break;
                }
            }
            (r, shown_at)
        }
    };
    // The paint remembered is one the edit recolored: a graph design's art that the item edit leaves out gives none.
    if let Some(shown) = shown_at.and_then(|(id, at)| s.doc().ok()?.doc.node(id).map(|n| proxy_paint(n, stroke, at))) {
        s.remember_paint(&shown);
    }
    Ok(r)
}

fn transparency(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "transparency.set";
    // An object of a dynamic symbol instance's art selected with Direct Selection: opacity and
    // blend change that object for this instance only. Isolate, knockout and Appearance `item` /
    // `target` stay refused here (they act on the instance or a stack row).
    if p.get("ids").is_none()
        && p.get("id").is_none()
        && p.get("item").is_none()
        && let Some((id, path)) = super::brushsym::selected_part(s)?
    {
        for key in ["isolate", "knockout", "knockoutShape", "aligned", "target"] {
            if p.get(key).is_some() {
                return Err(super::brushsym::part_only());
            }
        }
        if p.get("opacity").is_none() && p.get("blend").is_none() {
            return Err(bad(C, "give opacity or blend"));
        }
        check_numbers(p, &["opacity"], C)?;
        let opacity = match p.get("opacity") {
            None => None,
            Some(Value::Null) => Some(None),
            Some(v) => Some(Some(v.as_f64().map(super::opacitymask::percent).ok_or_else(|| bad(C, "opacity must be a number"))?)),
        };
        let blend = match p.get("blend") {
            None => None,
            Some(Value::Null) => Some(None),
            Some(b) => {
                let name = b.as_str().ok_or_else(|| bad(C, "blend must be a mode name"))?;
                Some(Some(BlendMode::parse(name).ok_or_else(|| bad(C, format!("unknown blend mode {b}")))?))
            }
        };
        super::brushsym::edit_part(s, id, &path, "Transparency", |o| {
            if let Some(op) = opacity {
                o.opacity = op;
            }
            if let Some(b) = blend {
                o.blend = b;
            }
        })?;
        return ok();
    }
    super::opacitymask::refuse_symbol_part_unnamed(s, p)?;
    let ids = super::opacitymask::transparency_targets(s, p)?;
    if ids.is_empty() {
        return Err(bad(C, "select objects or give ids"));
    }
    // The object-level opacity, blend mode, isolation and knockout of a graph design's art in a series are drawn again
    // from the design too.
    let ids = super::appearance::without_design_art(&s.doc()?.doc, ids, C)?;
    // While an opacity mask is edited the selection is its art, so the Appearance panel's active
    // item (a row of that art) doesn't stand for the masked object.
    let editing_mask = p.get("ids").is_none() && p.get("id").is_none() && s.doc()?.doc.mask_edit.is_some();
    let item = if editing_mask && p.get("item").is_none() { ItemTarget::Top } else { item_target(s, p, C)? };
    let mut q = p.as_object().cloned().unwrap_or_default();
    q.remove("item");
    q.remove("id");
    q.insert("ids".into(), json!(ids.iter().map(|id| id.0).collect::<Vec<_>>()));
    // The gradient mesh points picked with the Mesh tool, Direct Selection or the Lasso take the
    // opacity themselves, so only the mesh around them fades (#4206, #4170, #4208).
    if matches!(item, ItemTarget::Top)
        && let Some(o) = q.get("opacity").and_then(Value::as_f64)
        && let Some(points) = picked_mesh_points_of(s, &ids)
    {
        q.remove("opacity");
        let opacity = f64::from(super::opacitymask::percent(o));
        let items: Vec<Value> = points.iter().map(|(id, v)| json!({"id": id.0, "points": v})).collect();
        s.execute("object.mesh.setPointColor", &json!({"items": items, "opacity": opacity}))?;
        if q.keys().all(|k| k == "ids") {
            return ok();
        }
    }
    let ItemTarget::Item { index, explicit, aligned } = item else {
        return s.execute("object.setProps", &Value::Object(q));
    };
    // Opacity and blend belong to the targeted fill/stroke; isolate/knockout/knockoutShape stay object-level.
    let item_ids = if editing_mask { ids } else { row_targets(s, p, C)? };
    if !editing_mask {
        super::appearance::check_part_rows(s, p, aligned || !explicit, C)?;
    }
    // The art of a graph design whose stack an explicit aligned item is a row of goes first: appearance.setItem leaves
    // it as it is and matches the other objects' rows against it.
    let of = if editing_mask || !explicit { None } else { super::appearance::design_reference(s, p, C)? };
    if explicit {
        let d = &s.doc()?.doc;
        // With such art, an object has the item when it has a row equal to the art's.
        let found = match of {
            Some(_) => super::appearance::rows_per_target(d, of, &item_ids, &[index], true).iter().any(|r| !r.is_empty()),
            None => item_ids.iter().any(|id| d.node(*id).is_some_and(|n| index < n.appearance.items.len())),
        };
        if !found {
            return Err(bad(C, format!("no appearance item {index}")));
        }
    }
    let item_ids = super::appearance::with_reference(of, &item_ids);
    let (opacity, blend) = (q.remove("opacity"), q.remove("blend"));
    if let Some(b) = &blend
        && b.as_str().and_then(BlendMode::parse).is_none()
    {
        return Err(bad(C, format!("unknown blend mode {b}")));
    }
    if let Some(k) = q.get("knockout")
        && vectorcraft_doc::Knockout::from_value(k).is_none()
    {
        return Err(bad(C, format!("unknown knockout state {k}")));
    }
    if opacity.is_some() || blend.is_some() {
        let ids: Vec<u64> = item_ids.iter().map(|id| id.0).collect();
        s.execute("appearance.setItem", &json!({ "index": index, "ids": ids, "opacity": opacity, "blend": blend, "aligned": aligned }))?;
    }
    if q.keys().all(|k| k == "ids") {
        return ok();
    }
    s.execute("object.setProps", &Value::Object(q))
}

#[cfg(test)]
mod tests;
