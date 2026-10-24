//! Edit → Edit Colors → Recolor Artwork: list the colors of the selection (or of the art Select All
//! takes when nothing is selected), reduce them to rows of
//! similar colours, and remap each row to a new colour by a recolour method. Colours are keyed by
//! their identity ([`ColorKey`]: model and exact values), so a CMYK colour stays CMYK and two
//! colours that only look alike stay apart.

use std::collections::{BTreeSet, HashMap};

use serde_json::{Value, json};
use vectorcraft_color::recolor::{ColorKey, Method, Preserve, Rng, RowMap, cluster};
use vectorcraft_color::{Color, Paint, Swatch};
use vectorcraft_doc::swatches::{node_colors, node_paints};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};

use super::colorcmds::{Recolor, Scope};
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "recolor.colors",
            "Artwork Colors",
            [],
            None,
            "{ids?: object ids (default: the selection, or with nothing selected the objects select.all takes, without the hidden and locked objects inside them)} → {colors: [{key: colour identity (\"rgb 255 0 0\", \"cmyk 0 100 100 0\", \"gray 40\", \"lab 55 60 -40\"), hex, count, swatch?: the global swatch it is linked to}]} unique colours used by the objects (fills, strokes, gradient stops, text, mesh points and the tiles of pattern fills and strokes), most used first",
            has_doc,
            colors
        ),
        cmd!(
            "recolor.apply",
            "Recolor Artwork",
            ["Edit", "Edit Colors"],
            None,
            "{map: rows [{from: [colour keys (recolor.colors) or \"#rrggbb\" (every colour shown as that hex)], to: colour (hex, key, {c,m,y,k} or {l,a,b}) | null, exclude?: false (the row's colours are kept)}] | {\"key or #rrggbb\": colour, …} (one row per colour), method?: \"exact\" (default)|\"preserveTints\"|\"scaleTints\"|\"tintsShades\"|\"hueShift\" (how a row's colours take its new colour, relative to the row's darkest, average or most saturated colour), limitTo?: swatch library id or name; or \"document\" (the document's swatches): new colours snap to its nearest colour, group?: colour group to rewrite, groupColors?: [colour] (what `group` is rewritten with; default: the rows' new colours, in order, excluded rows left out), rename?: the group's new name, includeImages?: true, includePatterns?: true} recolour the selection (gradients, text, meshes, image pixels and pattern tiles included; each new colour keeps the model of the one it replaces (model?: \"keep\" default | \"new\" (the new colour's own model, so a CMYK colour replacing RGB art gives CMYK art) | \"document\" (the document's colour mode)), except a library swatch's with limitTo: art takes the swatch itself, linked to a global or spot colour at its tints, and the swatch joins the document's swatches; art linked to a global swatch of `group` that is rewritten with its row's new color stays linked to it, at its tint, unless the row converts the color into the art's model; other recolored art, and art in a row linked to a global swatch of `group` that is rewritten with another color, keeps its row's new color and loses its swatch link) and the group, as one undo step → {changed, swatches: [names added to the document's swatches], relinked?: with group, the other paints linked to its global swatches that took their new colors, note?: set when a colour kept a model other than its new colour's (see model)}",
            has_doc,
            apply
        ),
        cmd!(
            query "recolor.reduce",
            "Reduce Colors",
            [],
            None,
            "{colors?: n (rows; default: one per colour, with the tints of a global swatch in its row), method?: as recolor.apply; default scaleTints (picks each row's colour), preserve?: {white?: true, black?: true, grays?: false} (left out of the rows), limitTo?: as recolor.apply (new colours snap to it), ids?: as recolor.colors} group the colors of the objects (default: as recolor.colors) into rows of similar colours (k-means in Lab, weighted by use) → {map: [{from: [keys, the row's colour first], to: key}], preserved: [keys], method}",
            has_doc,
            reduce
        ),
        cmd!(
            query "recolor.randomize",
            "Randomize Colors",
            [],
            None,
            "{map: rows as recolor.apply, order?: true (shuffle the new colours among the rows that aren't excluded), saturationBrightness?: false (random saturation and brightness, hue kept), seed?: 0} → {map: the rows with their new colours as keys}",
            always,
            randomize
        ),
    ]
}

/// A colour the art uses: how often, and the global swatch it is linked to.
struct Used {
    color: Color,
    count: usize,
    swatch: Option<String>,
}

/// The unique colours of the objects `ids` and the objects inside them, most used first (ties by
/// key): fills, strokes, gradient stops, text, mesh points and the tiles of the patterns used as
/// fills or strokes. With `only`, an object not in it is left out with everything it holds.
fn used_colors(d: &Document, ids: &[NodeId], only: Option<&BTreeSet<NodeId>>) -> Vec<Used> {
    fn read<'a>(n: &'a Node, only: Option<&BTreeSet<NodeId>>, out: &mut Vec<&'a Node>) {
        if only.is_some_and(|o| !o.contains(&n.id)) {
            return;
        }
        out.push(n);
        for c in n.children().into_iter().flatten() {
            read(c, only, out);
        }
    }
    let mut index: HashMap<ColorKey, usize> = HashMap::new();
    let mut used: Vec<(ColorKey, Used)> = vec![];
    let mut count = |c: &Color, link: Option<&str>| {
        let k = ColorKey::of(c);
        let i = *index.entry(k).or_insert_with(|| {
            used.push((k, Used { color: *c, count: 0, swatch: None }));
            used.len() - 1
        });
        let u = &mut used[i].1;
        u.count += 1;
        if u.swatch.is_none() {
            u.swatch = link.filter(|l| d.swatch(l).is_some_and(|w| w.global && w.paint.color().is_some())).map(str::to_string);
        }
    };
    // The patterns used as fills or strokes: their tiles count once each.
    let mut patterns: Vec<&str> = vec![];
    let mut nodes = vec![];
    for n in ids.iter().filter_map(|id| d.node(*id)) {
        read(n, only, &mut nodes);
    }
    for m in nodes {
        for p in node_paints(m) {
            match p {
                Paint::Solid { color, swatch, .. } => count(color, swatch.as_deref()),
                Paint::Gradient(g) => g.gradient.stops.iter().for_each(|s| count(&s.color, s.swatch.as_deref())),
                Paint::Pattern { pattern, .. } if !patterns.contains(&pattern.as_str()) => patterns.push(pattern),
                _ => {}
            }
        }
        if let NodeKind::Mesh(mesh) = &m.kind {
            mesh.points.iter().for_each(|p| count(&p.color, None));
        }
    }
    for def in patterns.iter().filter_map(|p| d.pattern(p)) {
        def.art.iter().for_each(|n| node_colors(n, &mut count));
    }
    used.sort_by(|a, b| b.1.count.cmp(&a.1.count).then(a.0.cmp(&b.0)));
    used.into_iter().map(|(_, u)| u).collect()
}

/// The objects whose colors `recolor.colors` and `recolor.reduce` read, for [`used_colors`]: `ids`
/// (each checked) or else the selection, with everything they hold; with nothing selected, the
/// objects Select All takes and the objects inside them, leaving out every hidden or locked object
/// with what it holds and the objects of hidden, locked and template layers (the second value
/// lists the objects read). Objects listed twice or inside another one listed are dropped, so
/// their colors count once.
fn scope_ids(s: &Session, p: &Value, cmd: &str) -> Result<(Vec<NodeId>, Option<BTreeSet<NodeId>>)> {
    let st = s.doc()?;
    let ids = match p.get("ids") {
        Some(_) => checked_ids_param(s, p, "ids", cmd)?,
        None if st.selection.is_empty() => {
            let only = super::select::selectable_ids(&st.doc, st.isolation);
            return Ok((super::select::selectable(&st.doc, st.isolation), Some(only)));
        }
        None => st.selection.objects.clone(),
    };
    let mut seen = BTreeSet::new();
    Ok((super::edit::outermost(&st.doc, ids).into_iter().filter(|id| seen.insert(*id)).collect(), None))
}

fn colors(s: &mut Session, p: &Value) -> Result<Value> {
    let (ids, only) = scope_ids(s, p, "recolor.colors")?;
    let st = s.doc()?;
    let v: Vec<Value> = used_colors(&st.doc, &ids, only.as_ref())
        .iter()
        .map(|u| {
            let mut v = json!({"key": key_str(&u.color), "hex": u.color.to_hex(), "count": u.count});
            if let Some(w) = &u.swatch {
                v["swatch"] = json!(w);
            }
            v
        })
        .collect();
    Ok(json!({ "colors": v }))
}

/// The `method` parameter (`default` without one).
fn method_param(p: &Value, cmd: &str, default: Method) -> Result<Method> {
    match str_param(p, "method") {
        None => Ok(default),
        Some(m) => Method::parse(m).ok_or_else(|| {
            let ids: Vec<&str> = Method::ALL.iter().map(|m| m.id()).collect();
            bad(cmd, format!("unknown method `{m}` ({})", ids.join("|")))
        }),
    }
}

fn key_str(c: &Color) -> Value {
    json!(ColorKey::of(c).to_string())
}

fn reduce(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "recolor.reduce";
    let method = method_param(p, C, Method::ScaleTints)?;
    let palette = super::swatchlib::limit_param(s, p, C)?;
    let def = Preserve::default();
    let flag = |k: &str, d: bool| p.get("preserve").map_or(d, |v| bool_or(v, k, d));
    let preserve = Preserve { white: flag("white", def.white), black: flag("black", def.black), grays: flag("grays", def.grays) };
    let (ids, only) = scope_ids(s, p, C)?;
    let st = s.doc()?;
    let (kept, used): (Vec<Used>, Vec<Used>) = used_colors(&st.doc, &ids, only.as_ref()).into_iter().partition(|u| preserve.keeps(&u.color));
    // Units that stay together: the tints of a global swatch, else one colour each.
    let mut units: Vec<Vec<&Used>> = vec![];
    for u in &used {
        match units.iter_mut().find(|g| u.swatch.is_some() && g[0].swatch == u.swatch) {
            Some(g) => g.push(u),
            None => units.push(vec![u]),
        }
    }
    let unit_colors = |g: &[&Used]| g.iter().map(|u| u.color).collect::<Vec<_>>();
    let rows: Vec<Vec<&Used>> = match p.get("colors").and_then(Value::as_u64).map(index) {
        Some(n) if n < units.len() => {
            let keys: Vec<Color> = units.iter().map(|g| method.key_color(&unit_colors(g)).unwrap_or_default()).collect();
            let weights: Vec<f32> = units.iter().map(|g| g.iter().map(|u| u.count as f32).sum()).collect();
            cluster(&keys, &weights, n).into_iter().map(|ix| ix.into_iter().flat_map(|i| units[i].iter().copied()).collect()).collect()
        }
        _ => units,
    };
    let map: Vec<Value> = rows
        .iter()
        .map(|row| {
            let mut from = unit_colors(row);
            let key = method.key_color(&from).unwrap_or_default();
            // The row's colour first.
            if let Some(i) = from.iter().position(|c| *c == key) {
                from[..=i].rotate_right(1);
            }
            let to = palette.as_ref().map_or(key, |pl| pl.nearest(key));
            json!({"from": from.iter().map(key_str).collect::<Vec<_>>(), "to": key_str(&to)})
        })
        .collect();
    Ok(json!({"map": map, "preserved": kept.iter().map(|u| key_str(&u.color)).collect::<Vec<_>>(), "method": method.id()}))
}

/// A current colour of a row: an exact colour, or every colour shown as a hex.
enum Match {
    Key(ColorKey),
    Hex(String),
}

impl Match {
    /// A colour key, a hex (`#rrggbb`) or a colour value.
    fn of(v: &Value) -> Option<Self> {
        if let Some(s) = v.as_str() {
            if let Some(k) = ColorKey::parse(s) {
                return Some(Match::Key(k));
            }
            return Color::from_hex(s).map(|c| Match::Hex(c.to_hex()));
        }
        color_value(v).map(|c| Match::Key(ColorKey::of(&c)))
    }
    fn color(&self) -> Color {
        match self {
            Match::Key(k) => k.color(),
            Match::Hex(h) => Color::from_hex(h).unwrap_or_default(),
        }
    }
    fn to_json(&self) -> Value {
        match self {
            Match::Key(k) => json!(k.to_string()),
            Match::Hex(h) => json!(h),
        }
    }
}

/// A row of the map: its current colours and its new colour; an excluded row keeps its colours.
struct Row {
    from: Vec<Match>,
    to: Option<Color>,
    exclude: bool,
}

impl Row {
    /// The new colour, unless the row is excluded.
    fn new_color(&self) -> Option<Color> {
        self.to.filter(|_| !self.exclude)
    }
}

/// The rows of a `map` parameter, in order: an array of `{from, to, exclude?}` or an object of
/// `current: new` pairs.
fn parse_rows(p: &Value, cmd: &str) -> Result<Vec<Row>> {
    let new = |v: Option<&Value>| -> Result<Option<Color>> {
        match v {
            None | Some(Value::Null) => Ok(None),
            Some(v) => color_value(v).map(Some).ok_or_else(|| bad(cmd, format!("bad new colour {v}"))),
        }
    };
    let cur = |v: &Value| Match::of(v).ok_or_else(|| bad(cmd, format!("bad current colour {v}")));
    match p.get("map") {
        Some(Value::Object(m)) => m.iter().map(|(k, v)| Ok(Row { from: vec![cur(&json!(k))?], to: new(Some(v))?, exclude: false })).collect(),
        Some(Value::Array(rows)) => rows
            .iter()
            .map(|r| {
                let from = match r.get("from") {
                    Some(Value::Array(a)) => a.iter().map(cur).collect::<Result<Vec<_>>>()?,
                    Some(v) => vec![cur(v)?],
                    None => return Err(bad(cmd, "each row needs `from`")),
                };
                Ok(Row { from, to: new(r.get("to"))?, exclude: bool_or(r, "exclude", false) })
            })
            .collect(),
        _ => Err(bad(cmd, "missing `map` (rows [{from, to}] or {current: new})")),
    }
}

/// The document swatch a row's new colour is: its name, whether it is global (art links to it,
/// with tints) and the colour art linked to it shows (a process colour's own colour).
#[derive(Clone)]
struct Link {
    name: String,
    global: bool,
    color: Color,
}

/// A global swatch of the color group `recolor.apply` rewrites: its name, the color it is
/// rewritten with and the color art linked to it shows afterwards.
struct GroupSwatch {
    name: String,
    color: Color,
    shown: Color,
}

/// A parsed map ready to recolour: the row of each current colour, and the swatch of each row's
/// new colour.
struct ColorMap {
    rows: Vec<RowMap>,
    links: Vec<Option<Link>>,
    /// The new color of each of `rows`.
    tos: Vec<Color>,
    /// The global swatches of the color group being rewritten.
    group: Vec<GroupSwatch>,
    keys: HashMap<ColorKey, usize>,
    hexes: HashMap<String, usize>,
    /// Set when a colour took a new colour of another model in its own model.
    converted: std::cell::Cell<bool>,
}

impl ColorMap {
    /// `links`: the swatch of each of `rows`' new colours (as many as `rows`, or none); `group`: the
    /// global swatches of the color group being rewritten.
    fn new(rows: &[Row], method: Method, own_model: bool, links: &[Option<Link>], group: Vec<GroupSwatch>) -> Self {
        let mut out =
            Self { rows: vec![], links: vec![], tos: vec![], group, keys: HashMap::new(), hexes: HashMap::new(), converted: Default::default() };
        for (n, r) in rows.iter().enumerate().filter(|(_, r)| !r.from.is_empty()) {
            let Some(to) = r.new_color() else { continue };
            let i = out.rows.len();
            let colors: Vec<Color> = r.from.iter().map(Match::color).collect();
            out.rows.push(RowMap::new(method, &colors, to).with_own_model(own_model));
            out.tos.push(to);
            out.links.push(links.get(n).cloned().flatten());
            for m in &r.from {
                match m {
                    Match::Key(k) => out.keys.entry(*k).or_insert(i),
                    Match::Hex(h) => out.hexes.entry(h.clone()).or_insert(i),
                };
            }
        }
        out
    }

    /// The row of current colour `c`.
    fn row_of(&self, c: Color) -> Option<usize> {
        self.keys.get(&ColorKey::of(&c)).or_else(|| if self.hexes.is_empty() { None } else { self.hexes.get(&c.to_hex()) }).copied()
    }

    fn map(&self, c: Color) -> Color {
        let Some(r) = self.row_of(c).and_then(|i| self.rows.get(i)) else { return c };
        if r.converts(c) {
            self.converted.set(true);
        }
        r.map(c)
    }

    /// Recolour a colour that can link to a swatch: a row whose new colour is a global or spot
    /// swatch links its colours to it (at their tints; shades and other hues stay unlinked), a
    /// process swatch gives its colour in its own model; a color linked to a global swatch of the
    /// rewritten group that takes the row's new color keeps its link (at its tint in the row) when
    /// the row gives it the new color in that color's model;
    /// other rows map the colour, which loses its link when it changes or when the group's rewrite
    /// would change it. True when anything changed.
    fn map_link(&self, c: &mut Color, link: &mut Option<String>, tint: &mut f32) -> bool {
        let Some(i) = self.row_of(*c) else { return false };
        let (Some(row), Some(l)) = (self.rows.get(i), self.links.get(i)) else { return false };
        if l.is_none()
            && let Some(g) = link.as_deref().and_then(|n| self.group.iter().find(|g| g.name == n))
            && self.tos.get(i) == Some(&g.color)
            && !row.converts(*c)
            && let Some(t) = row.tint(*c)
        {
            let t = t.clamp(0.0, 1.0);
            let shown = g.shown.tinted(t);
            let changed = *tint != t || *c != shown;
            (*c, *tint) = (shown, t);
            return changed;
        }
        let new = match (l, row.tint(*c)) {
            (Some(l), Some(t)) if l.global => {
                let t = t.clamp(0.0, 1.0);
                let shown = l.color.tinted(t);
                let changed = link.as_deref() != Some(l.name.as_str()) || *tint != t || *c != shown;
                (*c, *link, *tint) = (shown, Some(l.name.clone()), t);
                return changed;
            }
            (Some(l), Some(t)) => l.color.tinted(t),
            _ => {
                if row.converts(*c) {
                    self.converted.set(true);
                }
                row.map(*c)
            }
        };
        // A color its row leaves as it is still loses its link to a swatch of the rewritten group
        // when the rewrite would give it another color.
        let rewritten = link.as_deref().and_then(|n| self.group.iter().find(|g| g.name == n)).is_some_and(|g| g.shown.tinted(*tint) != new);
        if new == *c && !rewritten {
            return false;
        }
        *c = new;
        *link = None;
        true
    }
}

fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "recolor.apply";
    let method = method_param(p, C, Method::Exact)?;
    let palette = super::swatchlib::limit_param(s, p, C)?;
    let mut rows = parse_rows(p, C)?;
    if let Some(pl) = &palette {
        for r in &mut rows {
            r.to = r.to.map(|c| pl.nearest(c));
        }
    }
    // With Limit to Library, a row's new colour is a swatch of the library: the art takes that
    // swatch (a spot colour stays a spot colour), which joins the document's swatches.
    let lib_swatches = match str_param(p, "limitTo").filter(|k| !k.is_empty()) {
        Some(k) => super::swatchlib::library_swatches(s, k).unwrap_or_default(),
        None => vec![],
    };
    let row_swatches: Vec<Option<Swatch>> =
        rows.iter().map(|r| r.new_color().and_then(|c| lib_swatches.iter().find(|w| w.paint.color() == Some(c)).cloned())).collect();
    let ids = s.doc()?.selection.objects.clone();
    let group = str_param(p, "group");
    if ids.is_empty() && group.is_none() {
        return Err(bad(C, "select artwork to recolour, or give a colour `group`"));
    }
    let scope = Scope { fill: true, stroke: true, ..Scope::of(p) };
    let label = if ids.is_empty() { "Edit Color Group" } else { "Recolor Artwork" };
    // `model`: keep each colour's model (default), take the new colour's own, or the document's.
    // A given `model` must be a string; only a missing one means "keep".
    let model = match p.get("model") {
        None => "keep",
        Some(Value::String(m)) => m.as_str(),
        Some(other) => return Err(bad(C, format!("bad model {other} (keep, new or document)"))),
    };
    let mut doc_model = None;
    let own_model = match model {
        "keep" => false,
        "new" => true,
        "document" => {
            let m = s.doc()?.doc.color_mode.model();
            doc_model = Some(m);
            for r in &mut rows {
                r.to = r.to.map(|c| c.in_model(m));
            }
            true
        }
        other => return Err(bad(C, format!("unknown model `{other}` (keep, new or document)"))),
    };
    // The colours a colour group takes follow the model too (`document` converts them).
    let in_doc_model = |c: Color| doc_model.map_or(c, |m| c.in_model(m));
    let new: Vec<Color> = match p.get("groupColors").and_then(Value::as_array) {
        Some(cs) => cs
            .iter()
            .map(|v| {
                color_value(v)
                    .map(|c| in_doc_model(palette.as_ref().map_or(c, |pl| pl.nearest(c))))
                    .ok_or_else(|| bad(C, format!("bad colour {v} in groupColors")))
            })
            .collect::<Result<_>>()?,
        None => rows.iter().filter_map(Row::new_color).collect(),
    };
    let rename = str_param(p, "rename");
    let (changed, edit, added, converted) = s.edit(label, |d, _| {
        // The rows' library swatches join the document (one it has with that name and colour is
        // used as it is).
        let mut added = vec![];
        let links: Vec<Option<Link>> = row_swatches
            .iter()
            .map(|w| {
                let w = w.as_ref()?;
                let name = match d.swatch(&w.name) {
                    Some(x) if x.paint == w.paint => w.name.clone(),
                    _ => {
                        let name = d.free_swatch_name(&w.name);
                        d.swatches.push(Swatch { name: name.clone(), ..w.clone() });
                        added.push(name.clone());
                        name
                    }
                };
                let color = d.global_color(&name).filter(|_| w.global).or(w.paint.color())?;
                Some(Link { name, global: w.global, color })
            })
            .collect();
        // The group's global swatches that its rewrite gives a color: art linked to one of them
        // whose row's new color is that color stays linked.
        let kept: Vec<GroupSwatch> = group
            .and_then(|g| d.swatch_groups.iter().find(|x| x.name == g))
            .map(|g| {
                g.swatches
                    .iter()
                    .zip(&new)
                    .filter(|(w, _)| w.global && !w.is_reserved())
                    .map(|(w, c)| GroupSwatch { name: w.name.clone(), color: *c, shown: d.linked_color(*c, w.spot) })
                    .collect()
            })
            .unwrap_or_default();
        let map = ColorMap::new(&rows, method, own_model, &links, kept);
        let f = |c: Color| map.map(c);
        let lf = |c: &mut Color, link: &mut Option<String>, tint: &mut f32| map.map_link(c, link, tint);
        let changed = Recolor::with_links(scope, &f, &lf).run(d, &ids);
        let edit = group.map(|g| super::swatch::rewrite_group(d, g, &new, rename, C)).transpose()?;
        Ok((changed, edit, added, map.converted.get()))
    })?;
    let mut out = json!({ "changed": changed, "swatches": added });
    if let Some(e) = edit {
        e.defaults(s);
        out["relinked"] = json!(e.relinked());
    }
    if converted {
        out["note"] = json!(
            "some colours kept their own colour model, so the new colour was converted into it and may look different; \
             pass model \"new\" (the new colour's model) or \"document\" (the document's colour mode) to change the model"
        );
    }
    Ok(out)
}

fn randomize(_: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "recolor.randomize";
    let rows = parse_rows(p, C)?;
    let mut rng = Rng::new(p.get("seed").and_then(Value::as_u64).unwrap_or(0));
    let mut news: Vec<Color> = rows.iter().filter_map(Row::new_color).collect();
    if bool_or(p, "order", true) {
        rng.shuffle(&mut news);
    }
    if bool_or(p, "saturationBrightness", false) {
        news = news.into_iter().map(|c| rng.saturation_brightness(c)).collect();
    }
    let mut news = news.into_iter();
    let map: Vec<Value> = rows
        .iter()
        .map(|r| {
            let to = if r.new_color().is_some() { news.next() } else { r.to };
            let mut row = json!({"from": r.from.iter().map(Match::to_json).collect::<Vec<_>>(), "to": to.as_ref().map(key_str)});
            if r.exclude {
                row["exclude"] = json!(true);
            }
            row
        })
        .collect();
    Ok(json!({ "map": map }))
}
