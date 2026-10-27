use super::convert::made_up;
use super::importer::{CSS_PX_PER_IN, PT_PER_IN};
use super::*;

/// What marks a nested `<g>` as a sublayer rather than a group: our export writes it on the layers
/// it writes inside layers. (Inkscape's `inkscape:groupmode="layer"` counts too.)
pub(crate) const SUBLAYER: &str = "data-vc-layer";

/// The Inkscape namespace (its layers and their labels).
pub(super) const INKSCAPE: &str = "http://www.inkscape.org/namespaces/inkscape";

/// The layer name on root element `root` (`<svg id="Main">`, or its `data-name`), as exports of
/// one-layer documents write it. An Inkscape root's id (`svg8`) is one Inkscape made up, not a name.
pub(super) fn root_layer_name(root: XNode) -> Option<String> {
    let id = root.attribute("id").filter(|_| root.attribute((INKSCAPE, "version")).is_none());
    root.attribute("data-name").or(id).map(str::trim).filter(|n| !n.is_empty()).map(str::to_string)
}

/// Is element `g` a `<g>` marked as a layer ([`SUBLAYER`])? Only one with an id counts: the id (or
/// the label beside it) is its name.
pub(super) fn marked_layer(g: XNode) -> bool {
    g.tag_name().name() == "g"
        && g.attribute("id").is_some_and(|id| !id.is_empty())
        && (g.has_attribute(SUBLAYER) || g.attribute((INKSCAPE, "groupmode")) == Some("layer"))
}

/// Turn the groups made of elements marked as layers (`marked`, see [`marked_layer`]) right inside
/// layer `l` into its sublayers, in its colour, and theirs in turn. A marked group anywhere else
/// (in a group, a mask, a symbol) stays a group: only layers hold layers.
pub(super) fn promote_sublayers(l: &mut Node, marked: &HashSet<NodeId>) {
    let NodeKind::Layer { color, children, .. } = &mut l.kind else { return };
    let color = *color;
    for c in children.iter_mut().filter(|c| marked.contains(&c.id)) {
        let c = Arc::make_mut(c);
        let NodeKind::Group { children, clip } = &mut c.kind else { continue };
        let mut sub = Node::layer(c.id, "", color);
        sub.set_clips(*clip);
        if let Some(ch) = sub.children_mut() {
            *ch = std::mem::take(children);
        }
        c.kind = sub.kind;
        promote_sublayers(c, marked);
    }
}

/// SVG's initial `font-size` (`medium`) in user units.
pub(crate) const DEFAULT_FONT_SIZE: f64 = 12.0;

/// How lengths become points, from the root `<svg>`'s `width` / `height`.
///
/// A pixel (and a unitless user unit) is a point, as we export, and absolute lengths keep their
/// physical size at 72 pt per inch: `font-size="12pt"` is 12 pt, `1in` is 72 pt. A root size in
/// absolute units (`width="210mm"`) is that physical size, and its user units are CSS pixels of it
/// (96 per inch, 0.75 pt each), so a drawing without a `viewBox` keeps its proportions. With one
/// side absolute and the other not (`width="96pt" height="96"`), both are: the unitless side is
/// CSS pixels too, so the drawing isn't stretched (#1010).
pub(super) struct RootUnits {
    /// Pixels per inch usvg (and the text pass) convert absolute lengths at.
    pub(super) dpi: f64,
    /// Points per user unit of the root, along both axes.
    pub(super) k: f64,
    /// Document units: those of the root `width` (pixels when it has none).
    pub(super) unit: Unit,
}

impl RootUnits {
    pub(super) fn of(root: XNode) -> Self {
        let unit = |name: &str| root.attribute(name).and_then(|v| svgtypes::Length::from_str(v.trim()).ok()).map(|l| l.unit);
        use svgtypes::LengthUnit as U;
        let physical = |u: Option<U>| matches!(u, Some(U::In | U::Cm | U::Mm | U::Pt | U::Pc));
        let (w, h) = (unit("width"), unit("height"));
        let (dpi, k) = if physical(w) || physical(h) { (CSS_PX_PER_IN, PT_PER_IN / CSS_PX_PER_IN) } else { (PT_PER_IN, 1.0) };
        let unit = match w {
            Some(U::Mm) => Unit::Millimeters,
            Some(U::Cm) => Unit::Centimeters,
            Some(U::In) => Unit::Inches,
            Some(U::Pt) => Unit::Points,
            Some(U::Pc) => Unit::Picas,
            _ => Unit::Pixels,
        };
        Self { dpi, k, unit }
    }
}

/// `'id' (feA, feB)` for each filter: its id and the primitives it uses, for warnings.
pub(super) fn filter_names(filters: &[Arc<usvg::filter::Filter>]) -> String {
    use usvg::filter::Kind as K;
    let names: Vec<String> = filters
        .iter()
        .map(|f| {
            let mut kinds: Vec<&str> = Vec::new();
            for p in f.primitives() {
                let k = match p.kind() {
                    K::Blend(_) => "feBlend",
                    K::ColorMatrix(_) => "feColorMatrix",
                    K::ComponentTransfer(_) => "feComponentTransfer",
                    K::Composite(_) => "feComposite",
                    K::ConvolveMatrix(_) => "feConvolveMatrix",
                    K::DiffuseLighting(_) => "feDiffuseLighting",
                    K::DisplacementMap(_) => "feDisplacementMap",
                    K::DropShadow(_) => "feDropShadow",
                    K::Flood(_) => "feFlood",
                    K::GaussianBlur(_) => "feGaussianBlur",
                    K::Image(_) => "feImage",
                    K::Merge(_) => "feMerge",
                    K::Morphology(_) => "feMorphology",
                    K::Offset(_) => "feOffset",
                    K::SpecularLighting(_) => "feSpecularLighting",
                    K::Tile(_) => "feTile",
                    K::Turbulence(_) => "feTurbulence",
                };
                if !kinds.contains(&k) {
                    kinds.push(k);
                }
            }
            let id = if f.id().is_empty() || made_up(f.id()) { String::new() } else { format!("'{}' ", f.id()) };
            format!("{id}({})", kinds.join(", "))
        })
        .collect();
    names.join(", ")
}

/// Inkscape's pages (`<inkscape:page>` in its `<sodipodi:namedview>`, in user units) as artboards
/// through `user` (user units → document points), named by their labels; `None` without any.
pub(super) fn inkscape_pages(xml: &roxmltree::Document, user: Affine) -> Option<Vec<vectorcraft_doc::Artboard>> {
    let num = |n: roxmltree::Node, a: &str| n.attribute(a).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite());
    let pages: Vec<vectorcraft_doc::Artboard> = xml
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "page" && n.tag_name().namespace() == Some(INKSCAPE))
        .filter_map(|p| {
            let (w, h) = (num(p, "width")?, num(p, "height")?);
            let (x, y) = (num(p, "x").unwrap_or(0.0), num(p, "y").unwrap_or(0.0));
            // A box too large to add up or to place (1e308 wide) is no page.
            let r = user.transform_rect_bbox(Rect::new(x, y, x + w, y + h));
            (w > 0.0 && h > 0.0 && r.is_finite()).then_some((p, r))
        })
        .take(1000)
        .enumerate()
        .map(|(i, (p, r))| vectorcraft_doc::Artboard {
            id: i as u32 + 1,
            name: p.attribute((INKSCAPE, "label")).filter(|l| !l.trim().is_empty()).map_or_else(|| format!("Artboard {}", i + 1), str::to_string),
            rect: r,
            locked: false,
            show_center_mark: false,
            show_cross_hairs: false,
            show_video_safe_areas: false,
            video_ruler_pixel_aspect_ratio: 1.0,
            color: None,
        })
        .collect();
    (!pages.is_empty()).then_some(pages)
}
