use super::columns::set_column;
use super::data::set_data;
use super::designs::design;
use super::edit::{create, set_type};
use super::markers::set_marker;
use super::{CommandSpec, cmd, has_doc, has_selection};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "graph.create",
            "Create Graph",
            [],
            None,
            "{type: column|stackedColumn|bar|stackedBar|line|area|scatter|pie|radar, x, y, width, height, series?: [..], categories?: [..], rows?: [[number|null]], csv?} → {id}; an empty CSV cell or a null is a blank value, a number in straight quotes is a label",
            has_doc,
            create
        ),
        cmd!(
            "graph.setData",
            "Data…",
            ["Object", "Graph"],
            None,
            "{id?, series?, categories?, rows?, csv?: first row = series labels (first cell empty); then one row per category: label; values…; an empty cell or a null is a blank value; a quoted number a label; a `|` in a label a line break, transpose?: true swaps rows and columns (Transpose row/column), switchXY?: true swaps each scatter series' y and x columns (Switch x/y)} replace the graph's data; no data → the current {csv, series, categories, rows}",
            has_selection,
            set_data
        ),
        cmd!(
            "graph.setType",
            "Type…",
            ["Object", "Graph"],
            None,
            "{id?, type?, seriesIndexes?: [index], columnWidth?: % (1–1000; past 100 columns overlap), clusterWidth?: % (1–1000), legend?: bool, markPoints?: bool, connectPoints?: bool, edgeToEdge?: bool (line graphs: true runs the lines across the whole plot, false puts the points at the centres of their categories), ticks?: n, axisMin?, axisMax?, valueAxis?: left|right|both (the bar graphs' value axis stays along the bottom), separateScales?: bool, rightTicks?: n, rightAxisMin?, rightAxisMax?, tickLength?: none|short|full, tickMarks?: n per division, rightTickLength?, rightTickMarks?, categoryTickLength?: none|short|full, categoryTickMarks?: n, ticksBetweenLabels?: bool, prefix?, suffix?, rightPrefix?, rightSuffix?: text around the value axis numbers, firstRowInFront?: bool, firstColumnInFront?: bool (which overlapping columns are drawn on top: the first category over the next within each series, the first series over the others), filledLines?: bool (Draw Filled Lines: connecting lines as 3 pt bands filled with the series colour), dropShadow?: bool (Add Drop Shadow: a translucent black shadow down and right of the columns, bars, lines and pie wedges), legendAcrossTop?: bool (Add Legend Across Top: the legend in rows above the plot), pieLegend?: none|standard|wedges (Legends in Wedges: the series labels inside their wedges), piePosition?: ratio|even|stacked (several pies sized by their totals, at one size (the default), or stacked), pieSort?: all|first|none (wedges largest first in each pie, in the first pie's order, or in data order (the default)); the query lists the pie options for pie graphs} change the graph type and options; with `seriesIndexes`, or (no `id`) with only series selected with Group Selection, `type` goes to those series only and `valueAxis` (left|right; both is refused) puts them on that value axis, while the other options still apply to the whole graph (Combine different graph types: column, stacked column, line and area mix, and so do bar and stacked bar; a series given the graph's type follows the graph again) (axisMin and axisMax together override the calculated value axis: exactly that range in `ticks` divisions, 5 when 0; with the value axis on both sides, separateScales gives the series on the right axis a scale of their own, set the same way with rightTicks, rightAxisMin and rightAxisMax); no options → the current ones",
            has_selection,
            set_type
        ),
        cmd!(
            "graph.design",
            "Design…",
            ["Object", "Graph"],
            None,
            "{new?: name|true (New Design: the selected art becomes a graph design, named \"New Design\" when true; put a rectangle at the back to set the size it draws at), rename?: name, to?: new name (the graphs drawing it follow), paste?: name (Paste Design: a copy of the design's art into the document, selected, to edit and save again), delete?: name|[names] (graphs drawing them go back to their default marks), ids?|id?: object ids (default: the selection), selectUnused?: bool (Select Unused: just the list)}; no params or selectUnused → {designs: [names], unused: [the names no graph draws with]}",
            has_doc,
            design
        ),
        cmd!(
            "graph.column",
            "Column…",
            ["Object", "Graph"],
            None,
            "{id?, seriesIndexes?: [index], design?: name|null, columnType?: vertical|uniform|repeating|sliding, each?: value (more than 0), fractions?: chop|scale} draw the columns or bars and the legend swatch of the series selected with Group Selection (or listed, or every series) with a graph design, its backmost object as its frame and its guides left out: vertical stretches it along the value axis to each column's length at the column's width, uniform scales it both ways to that length, centered on the column, repeating stacks copies out from the zero line that each stand for `each` and cuts off the last one where the value ends (chop) or scales it down to fit (scale) (vertical when one copy would be longer than the canvas), and sliding scales the part above its horizontal guide line (in bar graphs, right of a vertical one) in proportion to the column's width and stretches the part below to fill the rest (vertical without a guide, or in a column too short for the part above); type in the design that holds a `%` followed by two digits shows each column's value in place of the `%` and the digits, the first digit giving the least number of places before the decimal point (zeros added in front; 0 = as many as the value needs) and the second digit giving the places after it (rounded half away from zero), at the type's own size where the design's copy puts it (a repeating design shows it once, where a whole copy ending at the value end puts it; type inside a group that clips, is hidden or has its own opacity, blend mode, isolation, knockout, appearance or opacity mask, or inside a blend, compound shape, envelope, repeat or intertwine, stays there and is filled in each copy, drawn with the design; type inside a symbol stays as typed), and is left out of the legend swatch (a design of such type alone gets the default swatch); the design keeps its own orientation for negative values, and columns drawn with a design cast no drop shadow; null = plain columns; options left out keep the series' shared ones, else vertical, 10 and chop. Column, stacked column, bar and stacked bar series. No design → {designs: [names], design: the selected series' design or null, columnType, each, fractions}",
            has_selection,
            set_column
        ),
        cmd!(
            "graph.marker",
            "Marker…",
            ["Object", "Graph"],
            None,
            "{id?, seriesIndexes?: [index], design?: name|null} draw the data points and legend swatch of the series selected with Group Selection (or listed, or every series) with a graph design, scaled so its backmost object fills the default marker's square; null = the default marker. Line, scatter and radar series. No design → {designs: [names], design: the selected series' design or null}",
            has_selection,
            set_marker
        ),
    ]
}
