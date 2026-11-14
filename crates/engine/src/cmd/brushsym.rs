//! Brushes (Brushes panel, Paintbrush, Expand) and Symbols (Symbols panel, symbolism tools).
//!
//! Brush definitions live in `vectorcraft-brush` and are stored in `Document::unknown["brushes"]`.
//! Symbol definitions are `Document::symbols`; symbols made here are *normalised*: the art is
//! centred on the origin and scaled (non-uniformly, strokes untouched) into the ±10 pt box that
//! `Node::geometric_bounds` assumes for instances, and each instance's `xf` scales it back. The
//! natural size is kept in `Document::unknown["symbolSizes"]` so new instances come out right.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_brush::{self as brush, Bristle, Brush, BrushKind};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{AppearanceItem, Document, FillLayer, Node, NodeId, NodeKind, Symbol, SymbolExportType, SymbolNineSlice, SymbolRegistration};
use vectorcraft_geom::{Affine, Point, Vec2};

use super::edit::{outermost, selected_roots};
use super::pathops::{remove_each, separate};
use super::swatch::{anchor, names_param};
use super::{
    CommandSpec, Result, Session, bad, check_numbers, cmd, color_param, find_command, has_doc, has_selection, id_param, named_ids, number_or, ok,
    str_param, switch_or,
};
use crate::EngineError;

/// Whether Reset Transformation changes a symbol instance.
pub use symbols::reset_transformation_changes;

mod brushes;
mod dynamic;
mod symbols;

pub(crate) use brushes::{apply_brush, brush_targets, stroke_paint};
use brushes::{
    brush_apply, brush_delete, brush_duplicate, brush_freehand, brush_get, brush_list, brush_move, brush_new, brush_options, brush_place_art,
    brush_remove, brush_replace_art, brush_selection_art, brush_set_current, brush_unused, expand_brush,
};
pub(crate) use dynamic::{edit_part, fit_parts, part_only, selected_part};
use dynamic::{symbol_instance_reset, symbol_instance_select_part, symbol_instance_set_appearance};
use symbols::selected_instances;
pub(crate) use symbols::{
    expand_instances_under, expandable_instances, instance_in_place, natural_size, normalize, place_xf, set_natural_size, store_symbol,
    stored_natural_size,
};
use symbols::{
    symbol_adjust, symbol_break_link, symbol_delete, symbol_duplicate, symbol_instance_reset_transform, symbol_list, symbol_new, symbol_options,
    symbol_place, symbol_replace, symbol_select_instances, symbol_set_current, symbol_sort, symbol_spray, symbol_unused, symbol_update,
};

const SIZES_KEY: &str = vectorcraft_doc::SYMBOL_SIZES;
pub(crate) const CURRENT_SYMBOL: &str = "currentSymbol";
/// Half the instance box `Node::geometric_bounds` uses for symbol instances.
const HALF: f64 = vectorcraft_doc::SYMBOL_HALF;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        // ----- brushes -----
        cmd!(query "brush.list", "Brushes", [], None, "{} → {brushes: [{name, type}], current}", has_doc, brush_list),
        cmd!(query "brush.get", "Brush Definition", [], None, "{name} → the brush definition (JSON)", has_doc, brush_get),
        cmd!(
            query "brush.selectionArt",
            "Brush Art",
            [],
            None,
            "{ids?|id?} → {art: the object brush.new takes as a new scatter or art brush's art and as a new pattern brush's side tile (several objects become one group), or null when nothing is selected or ids name no object}",
            has_doc,
            brush_selection_art
        ),
        cmd!(
            "brush.placeArt",
            "Place Brush Art",
            [],
            None,
            "{name, x?, y?} place a copy of the art a scatter, art or pattern brush is made of, centered on (x, y) (default: the center of the first artboard), and select it, as dragging the brush from the Brushes panel onto an empty part of the canvas does; a pattern brush places its tiles left to right (side, outer corner, inner corner, start, end; the ones it has art for) 10 pt apart, each in a group named Side Tile, Outer Corner Tile, Inner Corner Tile, Start Tile or End Tile, with an unpainted rectangle at the back of a tile that has a repeat frame marking that frame; a calligraphic or bristle brush is refused → {id}",
            has_doc,
            brush_place_art
        ),
        cmd!(
            "brush.replaceArt",
            "Replace Brush Art",
            [],
            None,
            "{name, ids?|id?, applyToStrokes?: true} make the selected art (several objects become one group, as in brush.new) the art of a scatter, art or pattern brush, keeping its other options, as Alt-dragging (Option-dragging on macOS) art onto the brush in the Brushes panel does; a pattern brush takes a group named for one of its tiles, or a group of such groups (as brush.placeArt places them), tile by tile (of two named for one tile, the frontmost), each with the repeat frame an unpainted rectangle at its back marks (other tiles stay as they are), and other art as its side tile; when strokes use the brush and applyToStrokes is false, the brush and its strokes stay as they were and a copy of the brush with the new art, named `<name> copy`, goes after it and becomes the current brush; otherwise the strokes redraw with the new art; one undo step; refused for a calligraphic or bristle brush, with nothing selected, and for art (or a tile) with no width → {name: the brush that has the new art}",
            has_doc,
            brush_replace_art
        ),
        cmd!(
            "brush.apply",
            "Apply Brush",
            [],
            None,
            "{name, ids?|id?} apply a brush to the strokes of the selected paths (adds a stroke if missing) and make it current → {ids}",
            has_doc,
            brush_apply
        ),
        cmd!(
            "brush.remove",
            "Remove Brush Stroke",
            [],
            None,
            "{ids?|id?} remove brushes from the selected strokes → {ids}",
            has_selection,
            brush_remove
        ),
        cmd!(
            "brush.setCurrent",
            "Current Brush",
            [],
            None,
            "{name: string|null} the brush the Paintbrush tool paints with (not an undo step)",
            has_doc,
            brush_set_current
        ),
        cmd!(
            "brush.new",
            "New Brush…",
            [],
            None,
            "{type: calligraphic|scatter|art|pattern|bristle, name? (trimmed; blank: New <Type> Brush; a bristle brush name, with the number a taken name gets, has at most 31 characters), params?: {…definition fields}, ids?|id?} new brush; art/scatter/pattern brushes take the selected art (pattern: side tile) unless params give it → {name}",
            has_doc,
            brush_new
        ),
        cmd!(
            "brush.delete",
            "Delete Brush",
            [],
            None,
            "{name} | {names: [name…]} delete brushes; strokes using them become plain strokes",
            has_doc,
            brush_delete
        ),
        cmd!(
            "brush.move",
            "Move Brush",
            [],
            None,
            "{name? | names?: [brush names of one type, in order], to?: index among the brushes of that type in brush.list order (they go before the brush of that type now there that isn't moving; default: after the last one), type?: calligraphic|scatter|art|pattern|bristle (the type they go into; default: theirs)} reorder brushes within their type, as one undo step (none when the order stays as it was); brushes of different types, or a type that isn't theirs, are refused (brushes move only within their type) → {moved}",
            has_doc,
            brush_move
        ),
        cmd!(
            "brush.duplicate",
            "Duplicate Brush",
            [],
            None,
            "{name, newName? (trimmed; blank: <name> copy)} | {names: [name…]} → {name} | {names}",
            has_doc,
            brush_duplicate
        ),
        cmd!(
            query "brush.unused",
            "Select All Unused",
            [],
            None,
            "{} → {names: [the brushes no stroke paints with, in library order]} (the Brushes panel selects them): strokes in the art (opacity masks included), symbols, graph designs, pattern tiles (with the tiles that pattern editing's Cancel puts back) and graphic styles count, and so do strokes in the art of a brush that is used",
            has_doc,
            brush_unused
        ),
        cmd!(
            "brush.options",
            "Brush Options…",
            [],
            None,
            "{name, params?: {…fields to change (brush.get shows them; values outside their ranges are clamped). calligraphic: angle −180..180° (counter-clockwise from level), roundness 0..100%, size 0..1296 pt, variation: [angle°, roundness%, size pt], modes: [fixed|random|pressure|tilt|bearing|rotation for angle, roundness, size] (pressure goes from value − variation at the lightest pen pressure to value + variation at the heaviest; tilt from value − variation with the pen lying flat to value + variation with it upright; bearing and rotation add variation × the pen's bearing (which way it leans, counter-clockwise from leaning right) or its barrel's counter-clockwise rotation over 180°; the angle turns on past ±180°). scatter: size [a, b] 1..10000%, spacing [a, b] 1..10000%, scatter [a, b] −1000..1000%, rotation [a, b] −180..180° (counter-clockwise), modes: [fixed|random|pressure|tilt|bearing|rotation for size, spacing, scatter, rotation] (fixed: a; random: between a and b; pressure: a at the lightest pen pressure to b at the heaviest; tilt: a with the pen lying flat to b upright; bearing and rotation: a at −180° to b at 180°), rotation_relative_to_path. art: width 1..1000%, width_mode: null (fixed)|pressure|tilt|bearing|rotation, width_max 1..1000% (pressure: width at the lightest pen pressure to width_max at the heaviest; tilt: width with the pen lying flat to width_max upright; bearing and rotation: width at −180° to width_max at 180°; half way on a stroke drawn without that input), scale: {mode: proportional|stretch|betweenGuides, start, end: 0..1 of the art length}, direction: leftToRight|rightToLeft|topToBottom|bottomToTop, flip_along, flip_across, overlap. pattern: scale 1..10000%, spacing 0..10000%, fit: stretch|addSpace|approximate, flip_along, flip_across, outer_corner and inner_corner: corner tile art (drawn as the corner that joins a side tile on its right to one below it) or null, outer_source and inner_source: what that kind of corner draws without its art: none|autoCentered|autoBetween|autoSliced|autoOverlap (a tile made from the side tile), or null (the other kind of corner's art); side_tile, outer_corner_tile, inner_corner_tile, start_tile, end_tile: a document pattern name, original, or none (side refuses none); corner tiles also take autoCentered|autoBetween|autoSliced|autoOverlap. Each tile also accepts {pattern: name} to select a swatch whose name is a reserved choice. scatter, art and pattern: colorization: {method: none|tints|tintsAndShades|hueShift, key: colour (hueShift: the art colour that becomes the stroke colour; without a key: the solid color the art's fills and strokes use most, black when they use none)}; colorization and the art brush's scale change only the fields given}, newName? (trimmed; blank: the name is kept; a new bristle brush name has at most 31 characters, and a longer name the brush has is kept)} edit a brush definition (strokes using it update) → {name}",
            has_doc,
            brush_options
        ),
        cmd!(
            "object.expandBrush",
            "Expand Brush Strokes",
            [],
            None,
            "{ids?|id?} replace brushed paths (in the selection) with groups of the brush art → {ids}",
            has_selection,
            expand_brush
        ),
        cmd!(
            "brush.freehand",
            "Paintbrush",
            [],
            None,
            "{…params as path.freehand, brush} paint a freehand stroke with a brush → {id}",
            has_doc,
            brush_freehand
        ),
        // ----- symbols -----
        cmd!(
            query "symbol.list",
            "Symbols",
            [],
            None,
            "{} → {symbols: [{name, instances, size: [w,h], registration: 0..8, exportType: movieClip|graphic, nineSlice, type: static|dynamic}], current}",
            has_doc,
            symbol_list
        ),
        cmd!(
            "symbol.new",
            "New Symbol…",
            [],
            None,
            "{name?, ids?|id?, registration?: 0..8 (9-point grid, 4 = centre; Symbol Options › Registration), exportType?: movieClip (default)|graphic, nineSlice?: bool, nineSliceGuides?: array, type?: static (default)|dynamic (Symbol Options › Symbol Type)} make a symbol from the selection and replace it with an instance; a frame of threaded text in the selection leaves its thread with the text it shows, which the symbol's art holds; nineSlice is Symbol Options › Enable Guides for 9-Slice Scaling, nineSliceGuides [left, top, right, bottom] as fractions of the natural size in from each edge, each -16..16 (below 0 or above 1 puts a guide outside the art), with left + right < 1 and top + bottom < 1 (default a quarter each); an instance of a symbol with guides keeps its corners' size when scaled, its edges stretch along their length and its center in both directions → {name, id, registration, exportType, nineSlice, type}",
            has_selection,
            symbol_new
        ),
        cmd!(
            "symbol.options",
            "Symbol Options…",
            [],
            None,
            "{name?, newName?, registration?: 0..8, exportType?: movieClip|graphic, nineSlice?: bool, nineSliceGuides?: array, type?: static|dynamic} rename a symbol (its instances and the graphics mapped onto 3D objects follow the new name) and/or set Symbol Options › Registration, Export Type, 9-slice guides (nineSliceGuides [left, top, right, bottom] fractions in from each edge, each -16..16, with left + right < 1 and top + bottom < 1) and Symbol Type (a dynamic symbol's instances may change the appearance of objects in its art and stay linked; making it static drops those changes); default name: current → {name, registration, exportType, nineSlice, type}",
            has_doc,
            symbol_options
        ),
        cmd!(
            "symbol.place",
            "Place Symbol Instance",
            [],
            None,
            "{name?, x?, y?} place an instance so its registration point is at (x, y) (default: artboard centre; name default: current) → {id}",
            has_doc,
            symbol_place
        ),
        cmd!(
            "symbol.breakLink",
            "Break Link to Symbol",
            [],
            None,
            "{ids?|id?} expand selected instances into plain art (with their appearance changes to a dynamic symbol), placed through the symbol's 9-slice guides when it has them, with the instance's opacity mask; a hidden or locked instance's art stays hidden or locked and is not selected → {ids}",
            has_selection,
            symbol_break_link
        ),
        cmd!(
            "symbol.edit",
            "Edit Symbol",
            [],
            None,
            "{id?, ids?: [id…] (instead of id), name?, x?, y?, artboard?: index=0 (the active artboard; the app passes it), viewCenter?: [x, y] (the app passes the view's center)} edit a symbol's art in symbol editing mode: the instance `id` (default: the selected instances, which must share one symbol) where it shows the art, else the symbol `name` (default: the current symbol) centered on (x, y) (default: the center of artboard `artboard`, or of the first artboard when there is no such artboard; in a document without artboards, viewCenter, else (0, 0)); every instance follows each edit and the instance stays linked (symbol.breakLink expands one); refused while a pattern or an opacity mask is edited → {name, layer, id?}",
            has_doc,
            super::symboledit::enter
        ),
        cmd!(
            "symbol.exitEditing",
            "Exit Symbol Editing Mode",
            [],
            None,
            "{} leave symbol editing mode; after an edit, select the instance it was entered from; leaving without having changed the art adds no undo step and restores the selection from before entry (object.exitIsolation also leaves it) → {name?}",
            has_doc,
            super::symboledit::leave
        ),
        cmd!(
            "symbol.update",
            "Redefine Symbol",
            [],
            None,
            "{name, ids?|id?} replace the symbol's art with the selection (instances update, keeping their appearance changes to the objects at paths the new art still has; the selection becomes an instance; a frame of threaded text in the selection leaves its thread with the text it shows, which the symbol's art holds) → {name, id}",
            has_selection,
            symbol_update
        ),
        cmd!(
            "symbol.delete",
            "Delete Symbol",
            [],
            None,
            "{name | names: [name…], expandInstances?: true} delete a symbol (or several, in one undo step); its instances are expanded (or deleted with false)",
            has_doc,
            symbol_delete
        ),
        cmd!("symbol.duplicate", "Duplicate Symbol", [], None, "{name, newName?} → {name}", has_doc, symbol_duplicate),
        cmd!(
            "symbol.replace",
            "Replace Symbol",
            [],
            None,
            "{name, ids?|id?} swap the symbol of the selected instances (an instance given a different symbol drops its appearance changes; one given its own symbol keeps them) → {ids}",
            has_selection,
            symbol_replace
        ),
        cmd!(
            "symbol.instance.setAppearance",
            "Symbol Instance Appearance",
            [],
            None,
            "{id?|ids?: [one id] (default: the one selected object), path: [i…] (the object in the symbol's art: child indices in paint order from the art's root down through its groups; [] the root), fill?: \"#rrggbb\"|{…paint params of paint.setFill: color, none, swatch, tint, gradient, keepModel}|null (null: the symbol's own again), stroke?: as fill, opacity?: 0..100|null, blend?: name|null} change the appearance of one object of a dynamic symbol instance's art, for that instance only: it stays linked, so editing the symbol reshapes it and keeps the change; refused for a static symbol, a path naming no object, or a fill or stroke other than null for an object that does not draw it (a guide, blend, envelope, Repeat, Intertwine group, gradient mesh or placed document; a symbol instance's stroke, and its fill unless a solid color or none, which stains its art; an image's fill or stroke without a Convert to Shape or Outline Object effect), with nothing of the call applied; opacity, blend and null are taken for every object → {id, overrides: how many objects the instance changes}",
            has_selection,
            symbol_instance_set_appearance
        ),
        cmd!(
            "symbol.instance.selectPart",
            "Select Symbol Instance Part",
            [],
            None,
            "{id, path: [i…] (as symbol.instance.setAppearance)} select one path, compound path or type object of a dynamic symbol instance's art, as a Direct Selection click on it does: the instance is selected alone, and until the selection changes paint.setFill and paint.setStroke (solid, none, gradient or pattern), transparency.set with opacity and/or blend, paint.invert and paint.complement on a solid color (without ids, id or item; type whose characters differ in color is refused) and appearance.copyFrom (the Eyedropper, without ids, reverse or append; options that pick up neither color are refused; a gradient or pattern fill or stroke is taken) change that object for this instance only; the gradient annotator commands, isolate/knockout on transparency.set, object.group, edit.cut and edit.clear among the commands that change the selected objects without ids are refused; edit.copy copies the instance whole, and selection exports, artboard.fitToSelection, object.liquify, object.isolate, library.add and select.save use the instance (docs/mcp.md); selectPart itself is refused for a static symbol, a path naming no object, a group, image or other kind of object, a hidden, locked or template object or one inside such a group, and a hidden or locked instance → {id, path}",
            has_doc,
            symbol_instance_select_part
        ),
        cmd!(
            "symbol.instance.reset",
            "Reset Symbol Instance",
            [],
            None,
            "{ids?|id?} drop the selected dynamic symbol instances' own appearance changes, so they show the symbol's appearance again → {ids: the instances that had changes}",
            has_selection,
            symbol_instance_reset
        ),
        cmd!(
            "symbol.instance.resetTransform",
            "Reset Transformation",
            [],
            None,
            "{ids?|id?} give the selected symbol instances (in groups too) their symbol's own scale, without rotation, shear or reflection, keeping the center of each one's bounding box in place, as one undo step; other objects, instances in perspective and instances already at their symbol's scale are left as they are, and no undo step is added when nothing changes; an id naming no object, or no symbol instance among the objects, is an error → {ids: the instances changed}",
            has_selection,
            symbol_instance_reset_transform
        ),
        cmd!(
            "symbol.setCurrent",
            "Current Symbol",
            [],
            None,
            "{name} the symbol the symbolism tools spray (not an undo step)",
            has_doc,
            symbol_set_current
        ),
        cmd!(
            query "symbol.unused",
            "Select All Unused",
            [],
            None,
            "{} → {names: [the symbols nothing places, in panel order]} (the Symbols panel selects them): instances in the art (opacity masks included), graphics mapped onto 3D objects (threeD.mapGraphic), graph designs, pattern tiles (with the tiles that pattern editing's Cancel puts back) and brush art count, and so do instances in the art of a symbol that is used",
            has_doc,
            symbol_unused
        ),
        cmd!(
            "symbol.sortByName",
            "Sort by Name",
            ["Window", "Symbols"],
            None,
            "{} sort the document's symbols by name, ignoring case (symbols whose names differ only in case keep their order), as one undo step; the current symbol stays current; no undo step is added when they are already in order",
            has_doc,
            symbol_sort
        ),
        cmd!(query "symbol.selectInstances", "Select All Instances", [], None, "{name} select every instance of a symbol → {count}", has_doc, symbol_select_instances),
        cmd!(
            "symbol.spray",
            "Symbol Sprayer",
            [],
            None,
            "{points: [[x,y]…], name?, radius?: 40, density?: 1..10 (5), scale?: 1, alt?: bool (remove instances)} spray instances into a symbol set → {id, count}",
            has_doc,
            symbol_spray
        ),
        cmd!(
            "symbol.adjust",
            "Symbolism Tool",
            [],
            None,
            "{tool: shift|scrunch|size|spin|stain|screen|style, points: [[x,y]…], radius?: 40, intensity?: 1..10 (5), alt?: bool, color? (#rrggbbaa and #rgba are refused), keepModel?: bool (as paint.setFill), style?: name} adjust instances near the points (selection, else all) → {count}",
            has_doc,
            symbol_adjust
        ),
    ]
}

fn ids_json(ids: &[NodeId]) -> Value {
    Value::Array(ids.iter().map(|i| Value::from(i.0)).collect())
}

pub(crate) fn roots(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    match named_ids(p)? {
        Some(mut v) => {
            // Explicit ids: only existing, non-layer objects, none inside another of them.
            let d = &s.doc()?.doc;
            v.retain(|id| d.node(*id).is_some_and(|n| !n.is_layer()));
            v.dedup();
            Ok(outermost(d, v))
        }
        None => selected_roots(s),
    }
}

fn name_param<'a>(p: &'a Value, cmd: &str) -> Result<&'a str> {
    str_param(p, "name").filter(|n| !n.trim().is_empty()).ok_or_else(|| bad(cmd, "missing `name`"))
}

/// Set a document preference outside the undo history.
fn set_unknown(s: &mut Session, key: &str, v: Option<Value>) -> Result<()> {
    let st = s.doc_mut()?;
    let d = Arc::make_mut(&mut st.doc);
    match v {
        Some(v) => d.unknown.insert(key.into(), v),
        None => d.unknown.remove(key),
    };
    Ok(())
}

/// The selection as one art node (a group for several objects), ids kept.
pub(crate) fn selection_art(s: &Session, p: &Value) -> Result<Option<Node>> {
    let d = &s.doc()?.doc;
    let ids = roots(s, p)?;
    let nodes: Vec<Node> = ids.iter().filter_map(|id| d.node(*id).cloned()).collect();
    Ok(match nodes.len() {
        0 => None,
        1 => nodes.into_iter().next(),
        _ => Some(Node::group(NodeId(0), nodes.into_iter().map(Arc::new).collect())),
    })
}
