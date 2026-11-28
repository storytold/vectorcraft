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
    CommandSpec, Result, Session, bad, check_numbers, cmd, color_param, find_command, has_doc, has_selection, named_ids, number_or, ok, str_param,
    switch_or,
};
use crate::EngineError;

/// Whether Reset Transformation changes a symbol instance.
pub use symbols::reset_transformation_changes;

/// Whether an object is a symbol set.
pub use symbols::is_symbol_set;

/// The Symbols panel's current symbol, as `symbol.list` answers it.
pub use symbols::current_symbol;

mod brushes;
mod dynamic;
mod symbolism;
mod symbols;

pub(crate) use brushes::{apply_brush, brush_targets, frame_marker, stroke_paint};
use brushes::{
    brush_apply, brush_delete, brush_duplicate, brush_freehand, brush_get, brush_list, brush_move, brush_new, brush_object_options, brush_options,
    brush_place_art, brush_remove, brush_replace_art, brush_selection_art, brush_set_current, brush_unused, expand_brush,
};
pub(crate) use dynamic::{edit_part, fit_parts, part_only, selected_part};
use dynamic::{symbol_instance_reset, symbol_instance_select_part, symbol_instance_set_appearance};
use symbolism::symbol_set_density;
use symbols::selected_instances;
pub(crate) use symbols::{
    Placement, expand_instances_under, expandable_instances, instance_in_place, natural_size, normalize, place_xf, set_natural_size, store_symbol,
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
            "{name, params?: {…fields to change (brush.get shows them; values outside their ranges are clamped). calligraphic: angle −180..180° (counter-clockwise from level), roundness 0..100%, size 0..1296 pt, variation: [angle°, roundness%, size pt], modes: [fixed|random|pressure|tilt|bearing|rotation|stylusWheel for angle, roundness, size] (pressure goes from value − variation at the lightest pen pressure to value + variation at the heaviest; tilt from value − variation with the pen lying flat to value + variation with it upright; bearing and rotation add variation × the pen's bearing (which way it leans, counter-clockwise from leaning right) or its barrel's counter-clockwise rotation over 180°; stylusWheel from value − variation at one end of an airbrush pen's finger wheel to value + variation at the other; the angle turns on past ±180°). scatter: size [a, b] 1..10000%, spacing [a, b] 1..10000%, scatter [a, b] −1000..1000%, rotation [a, b] −180..180° (counter-clockwise), modes: [fixed|random|pressure|tilt|bearing|rotation|stylusWheel for size, spacing, scatter, rotation] (fixed: a; random: between a and b; pressure: a at the lightest pen pressure to b at the heaviest; tilt: a with the pen lying flat to b upright; bearing and rotation: a at −180° to b at 180°; stylusWheel: a at one end of an airbrush pen's finger wheel to b at the other), rotation_relative_to_path, hue_jitter 0..180° (each copy turns the hue of its solid colors, after the colorization, by its own amount up to this far either way, the same on every redraw; colors without a hue stay; 0: off), saturation_jitter 0..100% (each copy moves the saturation of its solid colors, after the colorization and the hue jitter, by its own amount up to this many points either way, keeping their hue and brightness, the same on every redraw; colors without a hue stay; 0: off), brightness_jitter 0..100% (each copy moves the brightness of its solid colors, after the colorization and the hue and saturation jitter, by its own amount up to this many points either way, keeping their hue and saturation, the same on every redraw; white, black and grays move too; 0: off). art: width 1..1000%, width_mode: null (fixed)|pressure|tilt|bearing|rotation|stylusWheel, width_max 1..1000% (pressure: width at the lightest pen pressure to width_max at the heaviest; tilt: width with the pen lying flat to width_max upright; bearing and rotation: width at −180° to width_max at 180°; stylusWheel: width at one end of the finger wheel to width_max at the other; half way on a stroke drawn without that input), scale: {mode: proportional|stretch|betweenGuides, start, end: 0..1 of the art length}, direction: leftToRight|rightToLeft|topToBottom|bottomToTop, flip_along, flip_across, overlap. pattern: scale 1..10000%, spacing 0..10000%, fit: stretch|addSpace|approximate, flip_along, flip_across, outer_corner and inner_corner: corner tile art (drawn as the corner that joins a side tile on its right to one below it) or null, outer_source and inner_source: what that kind of corner draws without its art: none|autoCentered|autoBetween|autoSliced|autoOverlap (a tile made from the side tile), or null (the other kind of corner's art); side_tile, outer_corner_tile, inner_corner_tile, start_tile, end_tile: a document pattern name, original, or none (side refuses none); corner tiles also take autoCentered|autoBetween|autoSliced|autoOverlap. Each tile also accepts {pattern: name} to select a swatch whose name is a reserved choice. scatter, art and pattern: colorization: {method: none|tints|tintsAndShades|hueShift, key: colour (hueShift: the art colour that becomes the stroke colour; without a key: the solid color the art's fills and strokes use most, black when they use none)}; colorization and the art brush's scale change only the fields given}, newName? (trimmed; blank: the name is kept; a new bristle brush name has at most 31 characters, and a longer name the brush has is kept)} edit a brush definition (strokes using it update) → {name}",
            has_doc,
            brush_options
        ),
        cmd!(
            "brush.objectOptions",
            "Options of Selected Object…",
            [],
            None,
            "{ids?|id?, params?: {…the fields brush.options takes that a path can set for itself: calligraphic angle (counter-clockwise from level), roundness, size, variation, modes; scatter size, spacing, scatter, rotation (counter-clockwise), modes, rotation_relative_to_path, colorization, hue_jitter, saturation_jitter, brightness_jitter; art width, width_mode, width_max, flip_along, flip_across, overlap, colorization; pattern scale, spacing, fit, flip_along, flip_across, colorization (values outside their ranges are clamped; a colorization object changes only the fields it gives; a null item of an array such as size: [40, null] keeps each stroke's own item, else the brush's)}, reset?: bool} set the brush settings of the brushed strokes of the selected paths on those strokes only, leaving the brush and its other strokes as they are (the Brushes panel's Options of Selected Object); params add to each stroke's own settings, a mode params does not set (no modes, or a null item) stays the one each stroke draws with when a changed variation or range would change it, a setting equal to the brush's own value is dropped, and reset: true clears them so the strokes draw the brush again; without params or reset nothing changes; one undo step, none when nothing changes. The brushes' own edits (brush.options) still reach the settings a stroke does not set, applying a brush (brush.apply, stroke.setAdvanced or a brush.freehand extend with another brush) or removing it clears them, and a renamed brush keeps them. Refused with nothing painted with a brush selected, for paths painted with brushes of different types, for a bristle brush, for a field the type does not take and for a value of the wrong kind → {ids: the brushed paths, changed: the paths whose settings changed, options: the first stroke's own settings}",
            has_doc,
            brush_object_options
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
            "{…params as path.freehand, brush?: name} paint a freehand stroke with a brush; without `brush` (or with null) the plain path is drawn; a `brush` that names no brush in the document, a `style` other than \"pencil\" or \"brush\", a `brush` for a path with no stroke (a continued or redrawn path without one, or a new filled \"pencil\" path while the stroke is None) and an unknown key are errors and nothing is drawn; the path and its brush are one undo step → {id}",
            has_doc,
            brush_freehand
        ),
        // ----- symbols -----
        cmd!(
            query "symbol.list",
            "Symbols",
            [],
            None,
            "{} → {symbols: [{name, instances, size: [w,h], registration: 0..8, exportType: movieClip|graphic, nineSlice, type: static|dynamic, alignToPixelGrid: bool}], current}",
            has_doc,
            symbol_list
        ),
        cmd!(
            "symbol.new",
            "New Symbol…",
            [],
            None,
            "{name?, ids?|id?, registration?: 0..8 (9-point grid, 4 = centre; Symbol Options › Registration), exportType?: movieClip (default)|graphic, nineSlice?: bool, nineSliceGuides?: array, type?: static (default)|dynamic (Symbol Options › Symbol Type), alignToPixelGrid?: bool (default false; Symbol Options › Align to Pixel Grid), keepArt?: false} make a symbol from the selection and replace it with an instance, or with keepArt: true leave the selection as ordinary art where it is, still selected, with id null; a frame of threaded text in the selection that becomes an instance leaves its thread with the text it shows, which the symbol's art holds; nineSlice is Symbol Options › Enable Guides for 9-Slice Scaling, nineSliceGuides [left, top, right, bottom] as fractions of the natural size in from each edge, each -16..16 (below 0 or above 1 puts a guide outside the art), with left + right < 1 and top + bottom < 1 (default a quarter each); an instance of a symbol with guides keeps its corners' size when scaled, its edges stretch along their length and its center in both directions; alignToPixelGrid keeps the symbol's instances on the pixel grid as symbol.options describes → {name, id, registration, exportType, nineSlice, type, alignToPixelGrid}",
            has_selection,
            symbol_new
        ),
        cmd!(
            "symbol.options",
            "Symbol Options…",
            [],
            None,
            "{name?, newName?, registration?: 0..8, exportType?: movieClip|graphic, nineSlice?: bool, nineSliceGuides?: array, type?: static|dynamic, alignToPixelGrid?: bool} rename a symbol (its instances and the graphics and Map Art maps on 3D objects follow the new name) and/or set Symbol Options › Registration, Export Type, 9-slice guides (nineSliceGuides [left, top, right, bottom] fractions in from each edge, each -16..16, with left + right < 1 and top + bottom < 1), Symbol Type (a dynamic symbol's instances may change the appearance of objects in its art and stay linked; making it static drops those changes) and Align to Pixel Grid (on: the horizontal and vertical straight segments of the paths in the symbol's art snap to whole points, or half points for an odd centered stroke, at the symbol's natural size, its stroke weights become whole points, and every instance placed at that size, reflected across one axis or not, moves to whole points and goes back to them after each move; a scaled, rotated (a half turn included) or sheared instance, and an instance in another symbol's art, is left where it is; the art snaps again after it is edited); alignToPixelGrid alone with the value the symbol already has changes nothing and adds no undo step; default name: current → {name, registration, exportType, nineSlice, type, alignToPixelGrid}",
            has_doc,
            symbol_options
        ),
        cmd!(
            "symbol.place",
            "Place Symbol Instance",
            [],
            None,
            "{name?, x?, y?} place an instance so its registration point is at (x, y) (default: artboard centre; name default: current), on top of the current layer, or in Draw Behind directly below the bottommost selected object (at the bottom of the current layer with nothing selected) → {id}",
            has_doc,
            symbol_place
        ),
        cmd!(
            "symbol.breakLink",
            "Break Link to Symbol",
            [],
            None,
            "{ids?|id?} expand selected instances into plain art (with their appearance changes to a dynamic symbol), placed through the symbol's 9-slice guides when it has them, with the instance's opacity mask and its own bounding box (square after Reset Bounding Box); a hidden or locked instance's art stays hidden or locked and is not selected → {ids}",
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
            "{name, ids?|id?, keepArt?: false} replace the symbol's art with the selection (instances update, keeping their appearance changes to the objects at paths the new art still has; the selection becomes an instance, or with keepArt: true stays ordinary art where it is, still selected, and id is null; a frame of threaded text in the selection that becomes an instance leaves its thread with the text it shows, which the symbol's art holds; art that holds an instance of the symbol (opacity masks included), directly or through the symbols it nests, is refused) → {name, id}",
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
            "{} → {names: [the symbols nothing places, in panel order]} (the Symbols panel selects them): instances in the art (opacity masks included), graphics and Map Art maps on 3D objects (threeD.mapGraphic, threeD.mapArt), graph designs, pattern tiles (with the tiles that pattern editing's Cancel puts back) and brush art count, and so do instances in the art of a symbol that is used",
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
            "{points: [[x,y]…] (a third value is a number or null and is not used), name?, radius?: 40, density?: 1..10 (5; a number outside 1..10 is taken as the nearer end), scale?: 1, alt?: bool (remove instances), attributes?: {scrunch?, size?, spin?, screen?, stain?, style?: average|userDefined (average)}, color? (#rrggbbaa and #rgba are refused), keepModel?: bool (as paint.setFill), style?: name} spray instances into a symbol set → {id, count}. Each of `attributes` says what a new instance takes: average, the mean of the selected set's instances within `radius` of the stroke point (their spacing along the stroke, size, rotation, opacity, and the stain or style of the nearest one that has one with an amount above 0, at the mean amount), and with none there the symbol's size, no rotation, full opacity and no stain or style; or userDefined: spacing from the symbol's size and `density`, the symbol's size times `scale`, turned the way the stroke runs there (not turned for one point), full opacity, stained in full with `color` (default: the fill color), and the fill of graphic style `style` (default: the first style after the default one) in full; a `style` the document does not have is then an error. The stain and the style are both the instance's fill, and the style replaces the stain. Another key or value in `attributes` is an error",
            has_doc,
            symbol_spray
        ),
        cmd!(
            "symbol.setDensity",
            "Symbol Set Density",
            [],
            None,
            "{from: 1..10, to: 1..10, ids?|id?} space the instances of the selected symbol sets (or those in `ids`) as Symbol Set Density `to` packs them, from the density each set was created at or last given (`from` for a set without one, such as one grouped by hand): each instance moves along the line from the middle of its set's instances, to (1.15 − 0.1·to) / (1.15 − 0.1·from) of its distance from it (each factor at least 0.1), as one undo step, and every set then has density `to`, also one whose instances do not move (one instance, or each locked); a higher density packs them closer, and a locked or hidden instance stays where it is. No undo step is added when no instance moves and every set already has density `to`. A `from` or `to` left out, not a number or outside 1..10, an id naming no object, or no symbol set among the objects is an error → {ids: the sets changed, count: the instances moved}",
            has_doc,
            symbol_set_density
        ),
        cmd!(
            "symbol.adjust",
            "Symbolism Tool",
            [],
            None,
            "{tool: shift|scrunch|size|spin|stain|screen|style, points: [[x,y,pressure?]…] (pen pressure 0..1, default 1), radius?: 40, intensity?: 1..10 (5), usePressure?: bool (each point's pressure is the intensity there, in place of `intensity`), method?: userDefined|average|random (userDefined; not used by shift), proportional?: bool (size; true), affectsDensity?: bool (size; false), alt?: bool, shift?: bool (shift, size, stain and style; false), color? (#rrggbbaa and #rgba are refused), keepModel?: bool (as paint.setFill), style?: name (a graphic style's; left out, or naming no style in the document, the first style after the default one)} adjust instances near the points (selection, else all) → {count}. `method` userDefined changes each instance by the tool's amount at its distance from the point; random gives each instance its own share of that amount for the whole stroke, from none to twice it (spin: a turn either way; style without `shift`: from none to the userDefined amount); average moves each instance's value toward the mean of the instances within `radius` of the point (scrunch: its distance from their middle toward their mean distance; size: the geometric mean of their sizes; spin: their mean direction; stain and style: their mean amount; screen: their mean opacity), and `alt` does not reverse it; average leaves an instance without a size as it is. With `proportional` false a drag resizes along its own direction, the width by the drag's horizontal share and the height by its vertical share (a level drag the width, an upright one the height, a 45° drag both alike; the first point takes the direction to the second) and one point resizes both; average resizes both whatever `proportional` is. With `affectsDensity` an instance also moves away from the point as it grows and toward it as it shrinks. The style goes on by the brush's strength at each sample, and a stroke's samples add up. With `alt` and without `shift`, userDefined and random style remove all of an instance's appearance items under the brush at once, whatever the brush's strength. `shift` is the tool's Shift form: shift brings the instances within `radius` of any point one place forward among their siblings (with `alt`, one place backward), once for the stroke, and does not move them; size keeps the instances where they are, `affectsDensity` included; stain and style keep each instance's amount and move its paint toward `color` or the style (stain by the tool's amount at its distance from the point, style by the brush's strength there), whatever `method` is, and leave an instance without a stain or style as it is, and `alt` has no effect. An instance counts only when it changes, and no undo step is added when none does (none under the brush, average with each already at the mean, each already at the tool's limit, or shift with nothing to move or change). A `method` that is none of the three, or a pressure that isn't a number, is an error",
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
