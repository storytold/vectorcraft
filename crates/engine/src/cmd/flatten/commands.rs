use super::options::flatten;
use super::presets::{presets_delete, presets_export, presets_import, presets_list, presets_save};
use super::report::preview;
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "object.flattenTransparency",
            "Flatten Transparency…",
            ["Object"],
            None,
            "{preset?: \"high\"|\"medium\"|\"low\" or a saved preset's name (see flattener.presets.list; default medium), balance?: 0..100 (raster/vector balance; 0 rasterizes everything), lineArtPpi?: 1..2400, gradientPpi?: 1..2400 (areas only gradients and meshes reach), textToOutlines?, strokesToOutlines?, clipComplexRegions? (clip images to the region outlines, else rectangles), antiAlias?, preserveAlpha? (composite over nothing instead of white), preserveOverprints? (areas showing one paint keep its colour and overprint; false clears overprints), options?: {the same keys}, ids?|id?} overlapping transparent objects become one group of flat-colour regions (CMYK colours composited ink by ink in CMYK documents), plus an image where gradients, patterns, images, masks or raster effects reach; objects without transparency stay; a frame of threaded text that is outlined or flattened leaves its thread with the text it shows → {ids, rasterized: images made, vector: regions made, options, warning? (an image in the art couldn't be decoded, as object.rasterize says)}",
            has_doc,
            flatten
        ),
        cmd!(
            query "flattener.presets.list",
            "Transparency Flattener Presets",
            [],
            None,
            "{} → {presets: [{name, builtIn, options}]} the built-in presets (High, Medium and Low Resolution), then the saved ones; any of these names works as `preset` wherever flattener options are taken",
            always,
            presets_list
        ),
        cmd!(
            query "flattener.presets.save",
            "Save Transparency Flattener Preset",
            [],
            None,
            "{name?: (default: a new \"Flattener Preset N\"), newName?: rename it, preset?: the preset to start from (default: the saved preset `name`, else medium), …options (the keys of object.flattenTransparency, at the top level or in `options`)} create or change a saved preset (built-in ones can't change) → {name, options, created}",
            always,
            presets_save
        ),
        cmd!(
            query "flattener.presets.delete",
            "Delete Transparency Flattener Preset",
            [],
            None,
            "{name} delete a saved preset (built-in ones stay) → {deleted: name}",
            always,
            presets_delete
        ),
        cmd!(
            query "flattener.presets.import",
            "Import Transparency Flattener Presets",
            [],
            None,
            "{path? | data?: file text | dataBase64?, replace?: false (replace saved presets of the same names; else the imported ones get a number)} add the presets of a .vcflattener file (as flattener.presets.export writes) to the saved ones → {imported: [names]}",
            always,
            presets_import
        ),
        cmd!(
            query "flattener.presets.export",
            "Export Transparency Flattener Presets",
            [],
            None,
            "{names?: [preset names, built-in ones too] (default: every saved preset), path?} write the presets as a .vcflattener file (JSON) → {path, count}; without path → {data: the file's text, count}",
            always,
            presets_export
        ),
        cmd!(
            query "flattener.preview",
            "Flattener Preview",
            [],
            None,
            "{highlight?: \"none\"|\"rasterizedRegions\" (areas the raster/vector balance rasterizes whole)|\"transparentObjects\"|\"allAffected\"|\"expandedPatterns\" (pattern art taking part)|\"outlinedStrokes\"|\"outlinedText\"|\"allRasterized\" (default none), overprints?: \"preserve\"|\"simulate\"|\"discard\" (simulate and discard flatten without preserveOverprints), preset?, …options as object.flattenTransparency, ids?|id?: (default: the whole document)} what flattening would do, without changing anything → {highlight, regions: [{bounds: {x, y, width, height}, id?}] (the highlighted objects, or areas), counts: {transparentObjects, allAffected, expandedPatterns, outlinedStrokes, outlinedText, rasterizedRegions, allRasterized, vectorRegions}, options}",
            has_doc,
            preview
        ),
    ]
}
