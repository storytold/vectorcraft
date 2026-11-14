//! A saved default palette and only the resources its paints need.
//!
//! The ordered panel records remain authoritative. Clipboard resource placement may rename
//! patterns, images and other definitions, but must not merge or number the panel's swatches.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde_json::json;
use vectorcraft_brush::Brush;
use vectorcraft_color::cms::{self, ColorSettings};
use vectorcraft_color::{Color, Paint, Swatch, SwatchGroup};
use vectorcraft_doc::text::{COMPOSITE_FONTS_MAX, COMPOSITE_NAME_MAX};
use vectorcraft_doc::{Appearance, CharClass, CompositeFont, Document, LayerColor, Node, NodeId, NodeKind};
use vectorcraft_geom::{Rect, shapes};
use vectorcraft_text::FontDb;

use super::clipboard::Clipboard;
use super::clipboard::resources::SwatchChoices;
use super::{EngineError, Result, Session};
use crate::SavedSwatchDefaults;

/// file.new calls apply before add_document activates its assigned profiles. Resource and brush
/// helpers use cms::active too, so bind the whole local operation and restore its caller's context.
/// use_document_profiles changes ACTIVE only; it leaves the working settings untouched.
struct ProfileScope {
    previous: ColorSettings,
}

impl ProfileScope {
    fn enter(d: &Document) -> Self {
        let previous = cms::active_settings();
        let (rgb, cmyk) = super::colormgmt::doc_profiles(d);
        cms::use_document_profiles(rgb.as_deref(), cmyk.as_deref());
        Self { previous }
    }
}

impl Drop for ProfileScope {
    fn drop(&mut self) {
        cms::use_document_profiles(Some(&self.previous.rgb), Some(&self.previous.cmyk));
    }
}

#[derive(Clone)]
struct Palette {
    swatches: Vec<Swatch>,
    groups: Vec<SwatchGroup>,
}

impl Palette {
    fn of(d: &Document) -> Self {
        Self { swatches: d.swatches.to_vec(), groups: d.swatch_groups.to_vec() }
    }

    fn iter(&self) -> impl Iterator<Item = &Swatch> {
        self.swatches.iter().chain(self.groups.iter().flat_map(|g| g.swatches.iter()))
    }

    fn install(&self, d: &mut Document) {
        d.swatches = self.swatches.clone().into();
        d.swatch_groups = self.groups.clone().into();
    }

    /// One independent seed per Paint also covers swatch-definition edges when the resource
    /// import deliberately leaves Clipboard.swatches empty.
    fn carriers(&self) -> Vec<Node> {
        self.iter()
            .map(|w| Node::path(NodeId(0), shapes::rectangle(Rect::new(0.0, 0.0, 1.0, 1.0)), Appearance::basic(w.paint.clone(), Paint::None, 0.0)))
            .collect()
    }

    fn rename_patterns(&mut self, renamed: &[serde_json::Value]) {
        let names: BTreeMap<&str, &str> =
            renamed.iter().filter(|r| r["kind"] == "pattern").filter_map(|r| Some((r["from"].as_str()?, r["to"].as_str()?))).collect();
        for w in self.swatches.iter_mut().chain(self.groups.iter_mut().flat_map(|g| g.swatches.iter_mut())) {
            if let Paint::Pattern { pattern, .. } = &mut w.paint
                && let Some(to) = names.get(pattern.as_str())
            {
                *pattern = (*to).to_string();
            }
        }
    }
}

fn failure(message: impl Into<String>) -> EngineError {
    EngineError::Other(message.into())
}

/// Build a minimal native capsule directly: importing here would resolve swatch colors/names
/// instead of preserving the original panel records.
fn capsule(source: &Document, palette: &Palette) -> Result<Document> {
    let copy = Clipboard::gather(source, palette.carriers());
    let mut d = Document::new_with_mode(1.0, 1.0, source.color_mode);
    // Keep the empty native layer/sentinel style; no source canvas art is copied.
    d.graphic_styles.retain(|style| style.id == 0);
    d.spot_use_lab = source.spot_use_lab;
    // The private Session activates its document. Keep the source's assigned context there;
    // apply imports only palette/resources and never assigns these profiles to the target.
    d.color_profiles = source.color_profiles.clone();
    d.images = copy.images;
    d.symbols = copy.symbols;
    d.patterns = copy.patterns;
    d.graphic_styles.extend(copy.graphic_styles);
    d.char_styles = copy.char_styles.into();
    d.para_styles = copy.para_styles.into();
    for (name, options) in copy.symbol_options {
        super::brushsym::set_natural_size(&mut d, &name, options.size);
        d.set_symbol_registration(&name, options.registration);
        d.set_symbol_export_type(&name, options.export_type);
        d.set_symbol_nine_slice(&name, options.nine_slice);
    }
    vectorcraft_brush::store(&mut d, &copy.brushes);
    palette.install(&mut d);
    d.fix_next_id();
    keep_used_composites(source, &mut d)?;
    Ok(d)
}

/// Text runs are not the only retained family names: styles and a graph's regeneration
/// settings keep them too. Use the same typed leaves for dependency collection and renaming.
fn each_family(d: &mut Document, visit: &mut impl FnMut(&mut String)) -> Result<()> {
    super::textstyles::each_text_mut(d, &mut |text| {
        for run in &mut text.runs {
            visit(&mut run.style.font_family);
        }
        Ok(())
    })?;
    for style in d.char_styles.iter_mut().chain(d.para_styles.iter_mut()) {
        if let Some(serde_json::Value::String(family)) = style.attrs.get_mut("font_family") {
            visit(family);
        }
    }
    let mut graphs = Vec::new();
    d.visit_stored(|node, _| {
        if node.extras().graph.is_some() {
            graphs.push(node.id);
        }
    });
    for id in graphs {
        if let Some(node) = d.stored_node_mut(id) {
            graph_families(node, visit);
        }
    }
    // Brush art is outside visit_stored. Text runs were handled by each_text_mut above;
    // visit only the remaining graph settings here and store only actual changes.
    fn brush_graphs(node: &mut Node, visit: &mut impl FnMut(&mut String)) {
        graph_families(node, visit);
        if let Some(mask) = node.extras_if_any().and_then(|extras| extras.mask.as_mut()) {
            brush_graphs(Arc::make_mut(&mut mask.art), visit);
        }
        for child in node.children_mut().into_iter().flatten() {
            brush_graphs(Arc::make_mut(child), visit);
        }
    }
    let mut brushes = vectorcraft_brush::library(d);
    let original = brushes.clone();
    for art in brushes.iter_mut().flat_map(|brush| brush.art_mut()) {
        brush_graphs(art, visit);
    }
    if brushes != original {
        vectorcraft_brush::store(d, &brushes);
    }
    Ok(())
}

fn graph_families(node: &mut Node, visit: &mut impl FnMut(&mut String)) {
    if let Some(graph) = node.extras_if_any().and_then(|extras| extras.graph.as_deref_mut()) {
        let labels = &mut graph.label_styles;
        for style in [&mut labels.values, &mut labels.categories, &mut labels.legend].into_iter().flatten() {
            visit(&mut style.font_family);
        }
    }
}

fn family_names(d: &mut Document) -> Result<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    each_family(d, &mut |name| {
        names.insert(name.to_lowercase());
    })?;
    Ok(names)
}

fn check_composites(fonts: &[CompositeFont], allow_installed_names: bool) -> Result<()> {
    if fonts.len() > COMPOSITE_FONTS_MAX {
        return Err(failure(format!("the default palette and profile need more than {COMPOSITE_FONTS_MAX} composite fonts")));
    }
    let db = FontDb::global();
    let mut names = BTreeSet::new();
    for font in fonts {
        font.check().map_err(failure)?;
        if !names.insert(font.name.to_lowercase()) {
            return Err(failure(format!("two composite fonts are named {:?}", font.name)));
        }
        if !allow_installed_names && db.has_installed_family(&font.name) {
            return Err(failure(format!("composite font {:?} names an installed family", font.name)));
        }
    }
    for font in fonts {
        for class in CharClass::ALL {
            let part = &font.parts.get(class).font;
            // Validate the prospective document, not another Session's active registry.
            if names.contains(&part.to_lowercase()) {
                return Err(failure(format!("composite font {:?} has a composite font as a part", font.name)));
            }
        }
    }
    Ok(())
}

/// Capture introduces no private font aliases: these are exact definitions of the active
/// source. Its private Session therefore registers only identical source meanings.
fn keep_used_composites(source: &Document, gathered: &mut Document) -> Result<()> {
    if source.composite_fonts.is_empty() {
        return Ok(());
    }
    let used = family_names(gathered)?;
    let fonts: Vec<CompositeFont> = source.composite_fonts.iter().filter(|font| used.contains(&font.name.to_lowercase())).cloned().collect();
    check_composites(&fonts, false)?;
    gathered.composite_fonts = fonts.into();
    Ok(())
}

fn composite_alias(base: &str, reserved: &BTreeSet<String>) -> Result<String> {
    let db = FontDb::global();
    let mut number = 2_u64;
    loop {
        let suffix = format!(" {number}");
        let stem: String = base.chars().take(COMPOSITE_NAME_MAX.saturating_sub(suffix.chars().count())).collect();
        let name = format!("{}{suffix}", stem.trim_end());
        if CompositeFont::clean_name(&name).as_deref() == Some(name.as_str())
            && !reserved.contains(&name.to_lowercase())
            && !db.has_installed_family(&name)
        {
            return Ok(name);
        }
        number = number.checked_add(1).ok_or_else(|| failure("no free name for a palette composite font"))?;
    }
}

/// Plan every name before touching the local target. Rename the capsule once, so resource
/// comparison/import sees the right families and profile type/definitions keep their meaning.
fn import_composites(saved: &mut Document, target: &mut Document) -> Result<()> {
    if saved.composite_fonts.is_empty() {
        return Ok(());
    }
    let source = saved.composite_fonts.to_vec();
    // A family installed since capture can occupy a saved name; planning aliases that
    // name before validating the combined target avoids dropping the saved definition.
    check_composites(&source, true)?;
    check_composites(&target.composite_fonts, false)?;
    let mut target_view = target.clone();
    let target_families = family_names(&mut target_view)?;
    let mut target_taken = target_families.clone();
    for font in target.composite_fonts.iter() {
        target_taken.insert(font.name.to_lowercase());
        target_taken.extend(CharClass::ALL.map(|class| font.parts.get(class).font.to_lowercase()));
    }
    let mut reserved = family_names(saved)?;
    reserved.extend(target_families.iter().cloned());
    for font in source.iter().chain(target.composite_fonts.iter()) {
        reserved.insert(font.name.to_lowercase());
        reserved.extend(CharClass::ALL.map(|class| font.parts.get(class).font.to_lowercase()));
    }
    let db = FontDb::global();
    let mut names = BTreeMap::new();
    let mut combined = target.composite_fonts.to_vec();
    let mut renamed = Vec::new();
    for font in source {
        let existing = CompositeFont::named(&target.composite_fonts, &font.name);
        let reused = existing.filter(|other| other.parts == font.parts);
        let name = match reused {
            Some(other) => other.name.clone(),
            None if target_taken.contains(&font.name.to_lowercase()) || db.has_installed_family(&font.name) => {
                composite_alias(&font.name, &reserved)?
            }
            None => font.name.clone(),
        };
        reserved.insert(name.to_lowercase());
        names.insert(font.name.to_lowercase(), name.clone());
        let copy = CompositeFont { name, parts: font.parts };
        if reused.is_none() {
            combined.push(copy.clone());
        }
        renamed.push(copy);
    }
    check_composites(&combined, false)?;
    each_family(saved, &mut |family| {
        if let Some(name) = names.get(&family.to_lowercase()) {
            *family = name.clone();
        }
    })?;
    saved.composite_fonts = renamed.into();
    target.composite_fonts = combined.into();
    Ok(())
}

/// Brush art is not in Document::visit_stored. Stage copies on one temporary layer so the
/// existing links.embed command can materialize its linked images/placed documents too.
fn stage_brushes(d: &mut Document, brushes: &[Brush]) -> Result<Vec<Vec<NodeId>>> {
    let mut ids = Vec::new();
    let mut nodes = Vec::new();
    for brush in brushes {
        let mut roots = Vec::new();
        for art in brush.art() {
            let copy = d.reid(art);
            roots.push(copy.id);
            nodes.push(Arc::new(copy));
        }
        ids.push(roots);
    }
    if !nodes.is_empty() {
        let mut layer = Node::layer(d.alloc_id(), "Palette resource copies", LayerColor::Preset(0));
        layer.children_mut().ok_or_else(|| failure("the palette resource layer has no children"))?.extend(nodes);
        d.layers.push(Arc::new(layer));
    }
    Ok(ids)
}

fn restore_brushes(d: &mut Document, brushes: &[Brush], ids: Vec<Vec<NodeId>>) -> Result<()> {
    if brushes.len() != ids.len() {
        return Err(failure("the staged palette brush count changed"));
    }
    // Embedding can adopt new brushes. Restore only the staged art into the current library,
    // retaining those imported definitions and all post-adoption brush metadata.
    let mut current = vectorcraft_brush::library(d);
    for (brush, roots) in brushes.iter().zip(ids) {
        let restored = current.iter_mut().find(|b| b.name == brush.name).ok_or_else(|| failure("the embedded palette brush is missing"))?;
        let art = restored.art_mut();
        if roots.len() != brush.art().len() || art.len() != roots.len() {
            return Err(failure("the staged palette brush art count changed"));
        }
        for (art, id) in art.into_iter().zip(roots) {
            let (copy, _) = d.stored_node(id).ok_or_else(|| failure("the embedded palette brush art is missing"))?;
            *art = (**copy).clone();
        }
    }
    vectorcraft_brush::store(d, &current);
    d.layers.clear();
    Ok(())
}

/// Capture is a query: every edit and link embedding happens in a private document/session.
/// The source's document, selection, undo journal and saved state are never changed.
pub(crate) fn capture(s: &Session) -> Result<String> {
    let source = s.doc()?;
    let _profiles = ProfileScope::enter(&source.doc);
    let palette = Palette::of(&source.doc);
    let d = capsule(&source.doc, &palette)?;
    let mut private = Session::new();
    private.prefs = s.prefs.clone();
    private.add_document(d, source.path.clone());
    private.doc_mut()?.links_from = source.links_from.clone();
    let mut passes = 0;
    let mut document_passes = 0;
    loop {
        // Expansion may import brushes or nested placed art. Stage the current library again
        // so every newly introduced dependency goes through the same embedding route.
        let brushes = vectorcraft_brush::library(&private.doc()?.doc);
        let staged = stage_brushes(Arc::make_mut(&mut private.doc_mut()?.doc), &brushes)?;
        let mut linked = Vec::new();
        let mut documents = false;
        private.doc()?.doc.visit_stored(|n, _| match &n.kind {
            NodeKind::Image(im) if im.link.is_some() => linked.push(n.id.0),
            NodeKind::PlacedDocument(_) => {
                linked.push(n.id.0);
                documents = true;
            }
            _ => {}
        });
        if linked.is_empty() {
            restore_brushes(Arc::make_mut(&mut private.doc_mut()?.doc), &brushes, staged)?;
            break;
        }
        // Sequential expansions do not share the native reader's recursion guard. Bound the
        // document waves by its supported depth, allowing one final image-only leaf wave.
        let depth = vectorcraft_doc::placed_document::MAX_DEPTH;
        if passes > depth || (documents && document_passes >= depth) {
            return Err(failure("the default swatches have cyclic or too deeply nested linked documents"));
        }
        let result = private.execute("links.embed", &json!({"ids": linked}))?;
        if result.get("missing").and_then(serde_json::Value::as_array).is_some_and(|missing| !missing.is_empty()) {
            return Err(failure("the default swatches need a linked file whose full contents are unavailable"));
        }
        passes += 1;
        document_passes += usize::from(documents);
        restore_brushes(Arc::make_mut(&mut private.doc_mut()?.doc), &brushes, staged)?;
    }
    let mut embedded = (*private.doc()?.doc).clone();
    // Embedding a placed document can introduce palette links of its own. Only names in the
    // captured panel remain global: removed definitions follow the normal unlink convention.
    palette.install(&mut embedded);
    refresh_references(&mut embedded, &palette)?;
    palette.install(&mut embedded);
    let d = capsule(&embedded, &palette)?;
    if d.images.values().any(vectorcraft_doc::ImageBlob::is_proxy) {
        return Err(failure("the default swatches need full image data, not a linked preview"));
    }
    let options = vectorcraft_format::SaveOptions { include_linked: true, precision: None, ..Default::default() };
    let bytes = vectorcraft_format::save_with(&d, &options).map_err(|e| failure(e.to_string()))?;
    Ok(vectorcraft_format::base64_encode(&bytes))
}

/// Refresh existing profile resources against their replacement palette, while keeping the
/// palette's own raw Color values separate for final restoration.
fn refresh_references(d: &mut Document, palette: &Palette) -> Result<()> {
    let mut colors: BTreeMap<String, Color> = palette.iter().filter_map(|w| Some((w.name.clone(), d.global_color(&w.name)?))).collect();
    let registration = vectorcraft_color::swatch::REGISTRATION;
    if let Some(color) = d.global_color(registration) {
        colors.insert(registration.into(), color);
    }
    let gradients: BTreeSet<String> = palette.iter().filter(|w| matches!(w.paint, Paint::Gradient(_))).map(|w| w.name.clone()).collect();
    let mut color_link = |color: &mut Color, link: &mut Option<String>, tint: &mut f32| {
        let Some(name) = link.as_deref() else { return false };
        match colors.get(name) {
            Some(base) => {
                let new = base.tinted(*tint);
                let changed = *color != new;
                *color = new;
                changed
            }
            None => {
                *link = None;
                true
            }
        }
    };
    let mut gradient_source = |paint: &Paint| {
        let mut next = paint.clone();
        if let Paint::Gradient(g) = &mut next
            && g.swatch.as_ref().is_some_and(|name| !gradients.contains(name))
        {
            g.swatch = None;
            Some(next)
        } else {
            None
        }
    };
    // This existing walker also keeps a Hue Shift brush's colorization key in step with its art.
    super::swatch::map_swatch_links(d, &mut color_link);
    super::swatch::map_all_paints(d, &mut gradient_source);
    // Document's paint walker visits character style attrs, not paragraph style attrs.
    for style in &mut d.para_styles {
        for key in ["fill", "stroke"] {
            let Some(value) = style.attrs.get(key) else { continue };
            let Ok(mut paint) = serde_json::from_value::<Paint>(value.clone()) else { continue };
            let changed = paint.map_links(&mut color_link);
            let source = gradient_source(&paint);
            if changed || source.is_some() {
                let next = source.unwrap_or(paint);
                style.attrs.insert(key.into(), serde_json::to_value(next).map_err(|e| failure(e.to_string()))?);
            }
        }
    }
    Ok(())
}

fn apply_custom(d: &mut Document, data: &str) -> Result<()> {
    let bytes = vectorcraft_format::base64_decode(data).ok_or_else(|| failure("the saved default swatches are not valid base64"))?;
    let mut saved = vectorcraft_format::load_file(&bytes).map_err(|e| failure(e.to_string()))?.doc;
    import_composites(&mut saved, d)?;
    let mut palette = Palette::of(&saved);
    palette.install(d);
    let mut copy = Clipboard::gather(&saved, palette.carriers());
    // All swatch paints are independent carrier seeds. Their dependency edges are already
    // covered, so resource import can leave the authoritative panel and its raw colors alone.
    copy.swatches.clear();
    let imported = copy.import_into(d, &SwatchChoices::default(), false)?;
    palette.rename_patterns(&imported.renamed);
    refresh_references(d, &palette)?;
    palette.install(d);
    d.fix_next_id();
    Ok(())
}

fn apply_factory(d: &mut Document) -> Result<()> {
    let (swatches, groups) = vectorcraft_color::default_swatches(d.color_mode.model());
    let palette = Palette { swatches, groups };
    palette.install(d);
    refresh_references(d, &palette)?;
    palette.install(d);
    Ok(())
}

/// The caller owns a newly constructed document. Replace only its palette/resources, after
/// its category/profile settings have been applied. On error even that local document is intact.
pub(crate) fn apply(d: &mut Document, defaults: &SavedSwatchDefaults) -> Result<()> {
    let _profiles = ProfileScope::enter(d);
    let mut next = d.clone();
    match defaults {
        SavedSwatchDefaults::Factory => apply_factory(&mut next)?,
        SavedSwatchDefaults::Custom { data } => apply_custom(&mut next, data)?,
    }
    *d = next;
    Ok(())
}
