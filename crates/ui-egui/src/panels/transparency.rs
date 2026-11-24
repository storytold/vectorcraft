//! Transparency panel: blend mode, opacity (field + slider popup), object/mask thumbnails with the
//! opacity-mask controls (make/release, link, clip, invert; Shift-click the mask to disable it,
//! Alt-click it to view only the mask), Isolate Blending, the three-state Knockout Group (on →
//! neutral → off) and Opacity & Mask Define Knockout Shape; the menu's Page Isolated Blending and
//! Page Knockout Group.

use std::cell::RefCell;
use std::sync::Arc;

use craft::behavior::IconId;
use craft::behavior::Mixed;
use craft::behavior::item_source::keyed;
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, CollectionEvent, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::display::{Divider, Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow, RowEvent, ThumbSize, ThumbTarget};
use craft::ui::menu::PanelRow;
use craft::ui::pickers::Select;
use craft::ui::primitives::{
    ContentPainterCx, IconSpec, LoadFailure, LoadedPicture, PaintShape, Priority, StrokeWidth, ThumbJob, ThumbRender, ThumbRequest, ThumbSource,
    ThumbnailOptions, Thumbnails,
};
use craft::ui::primitives::{Revision, ThumbPaint};
use craft::ui::runtime::commands::{CommandRef, CommandState, CommandStates};
use craft::ui::runtime::env::UnitKind;
use craft::ui::text::NumberField;
use egui::{Color32, Ui};
use serde_json::{Value, json};
use vectorcraft_doc::{Document, Knockout, Node, NodeId, OpacityMask};
use vectorcraft_engine::cmd::TransparencyInfo;

use super::{current_paints, current_transparency, live_run, selection_len};
use crate::VectorcraftApp;
use crate::edits::{EditPath, apply_edits};
use crate::fields::blend_entries;

/// The side of the mask row's thumbnails, in points.
// PLAN: should come from the `size.thumb.lg` token; the row's `ThumbSize::Large` gives the rect, this only sizes the render.
const THUMB_POINTS: f32 = 48.0;

/// The command ids of the menu's own rows.
const THUMBNAILS: &str = "ui.transparency.thumbnails";
const OPTIONS: &str = "ui.transparency.options";

/// What the panel remembers between passes: which blocks the menu hid.
#[derive(Clone, Debug, Default)]
pub(crate) struct TransparencyPanel {
    /// The thumbnails and the opacity-mask controls are hidden.
    pub(crate) hide_thumbs: bool,
    /// The Isolate Blending and knockout options are hidden.
    pub(crate) hide_options: bool,
}

/// The cache key of [`state`]: document, revision, selection size and ends, and the picked mesh points.
pub(crate) type StateKey = (u64, u64, usize, Option<NodeId>, Option<NodeId>, Option<Vec<(NodeId, Vec<usize>)>>);

/// The first target of `info` as the document holds it.
fn first_node(app: &VectorcraftApp, info: &TransparencyInfo) -> Option<Arc<Node>> {
    let st = app.session.active()?;
    info.ids.first().and_then(|id| super::layers::node_arc(&st.doc, *id))
}

/// The panel's values ([`vectorcraft_engine::Session::transparency_info`]) and its first target
/// (the selection, or the object whose mask is being edited), recomputed only when the document
/// or the selection changes, or other gradient mesh points are picked. The target is the `Arc` the
/// document holds.
pub(crate) fn state(app: &mut VectorcraftApp) -> (TransparencyInfo, Option<Arc<Node>>) {
    let Some(st) = app.session.active() else { return Default::default() };
    let sel = &st.selection.objects;
    let key: StateKey = (st.uid, st.revision, sel.len(), sel.first().copied(), sel.last().copied(), app.session.mesh_points_picked());
    if let Some((k, info, n)) = &app.canvas.transparency_state
        && *k == key
    {
        return (info.clone(), n.clone());
    }
    let info = app.session.transparency_info();
    let first = first_node(app, &info);
    app.canvas.transparency_state = Some((key, info.clone(), first.clone()));
    (info, first)
}

/// Whether the object-only controls apply: while a mask is edited, or when the Appearance panel's
/// active row is the object itself (a fill or stroke has no mask, isolation or knockout).
fn object_controls(app: &VectorcraftApp, editing: bool) -> bool {
    editing || app.session.appearance_item().is_none()
}

/// The key of one mask-row thumbnail: the masked object, the background it sits on and which picture it is.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct MaskKey {
    uid: u64,
    /// The object that carries the mask.
    node: NodeId,
    bg: Option<[u8; 4]>,
    /// True: the mask's art. False: the object drawn without its mask.
    own_mask: bool,
}

/// The mask row's pictures, rendered as jobs so a drag never waits for them.
pub(crate) struct MaskThumbs<'a> {
    doc: &'a Arc<Document>,
    revision: u64,
}

thread_local! {
    static RENDERER: RefCell<vectorcraft_render::Renderer> = RefCell::new(vectorcraft_render::Renderer::new());
}

/// The picture of `obj` (without its mask) or of its mask's art, `px` on a side; `None` when there is nothing to draw.
fn thumb_image(doc: &Arc<Document>, obj: &Node, own_mask: bool, px: u32, bg: Option<[u8; 4]>) -> Option<egui::ColorImage> {
    let bare;
    let n: &Node = if own_mask {
        &obj.extras().mask.as_deref()?.art
    } else if obj.extras().mask.is_some() {
        bare = {
            let mut bare = obj.clone();
            bare.extras_mut().mask = None;
            bare
        };
        &bare
    } else {
        obj
    };
    let img = RENDERER.with(|r| r.borrow_mut().render_node_thumbnail_with(doc, n, px.max(8), bg, &super::layers::thumb_options()))?;
    Some(crate::canvas::color_image(&img))
}

impl ThumbSource<MaskKey> for MaskThumbs<'_> {
    fn revision(&self, key: &MaskKey, _: &ThumbRequest) -> Revision {
        Revision::of(&(self.revision, key.node, key.bg, key.own_mask))
    }

    fn render(&self, key: &MaskKey, request: &ThumbRequest) -> ThumbRender {
        let Some(obj) = super::layers::node_arc(self.doc, key.node) else { return ThumbRender::Blank };
        let (doc, px, bg, own_mask) = (self.doc.clone(), request.side, key.bg, key.own_mask);
        let job: ThumbJob = Box::new(move |_| Ok(thumb_image(&doc, &obj, own_mask, px, bg).map_or(LoadedPicture::Blank, LoadedPicture::Image)));
        let _: Option<LoadFailure> = None;
        ThumbRender::Later(job)
    }
}

/// The command a click on the mask thumbnail runs: Shift disables or enables the mask, Alt views only
/// the mask, a plain click edits it (none while it is being edited).
fn mask_click(m: &OpacityMask, id: NodeId, mods: craft::keymap::Mods, editing: bool) -> Option<(&'static str, Value)> {
    if mods.shift {
        Some((if m.disabled { "transparency.enableOpacityMask" } else { "transparency.disableOpacityMask" }, json!({})))
    } else if mods.alt {
        Some(("transparency.viewOpacityMask", json!({"id": id.0})))
    } else if !editing {
        Some(("transparency.editOpacityMask", json!({"id": id.0})))
    } else {
        None
    }
}

/// The one-row list of the object's and the mask's thumbnails with the mask's link; the events it reported.
fn mask_row(
    app: &VectorcraftApp,
    ui: &mut Ui,
    n: Option<&Arc<Node>>,
    mask: Option<&OpacityMask>,
    editing: bool,
    object: bool,
) -> Vec<CollectionEvent<(), RowEvent>> {
    let Some(st) = app.session.active() else { return vec![] };
    let source = MaskThumbs { doc: &st.doc, revision: st.revision };
    let store = Thumbnails::<MaskKey>::in_context(ui.ctx(), "vc-mask-thumbs", ThumbnailOptions::new());
    let request = ThumbRequest::for_cell(THUMB_POINTS, ui.ctx().pixels_per_point());
    let state_of = |ui: &Ui, node: NodeId, bg, own_mask| {
        store.state(ui.ctx(), &source, &MaskKey { uid: st.uid, node, bg, own_mask }, &request, Priority::VISIBLE)
    };
    let bg = mask.map(|m| if m.clip != m.invert { [0, 0, 0, 255] } else { [255, 255, 255, 255] });
    let object_state = n.map(|n| state_of(ui, n.id, None, false));
    let mask_state = n.zip(mask).map(|(n, _)| state_of(ui, n.id, bg, true));
    let fill = n.and_then(|_| current_paints(app).0.color()).map(|c| super::c32(&c));
    let paper =
        ThumbPaint::default().paper(craft::ui::style::DataColor::new(if bg == Some([0, 0, 0, 255]) { Color32::BLACK } else { Color32::WHITE }));
    let plain = ThumbPaint::default();
    let (has_mask, disabled) = (mask.is_some(), mask.is_some_and(|m| m.disabled));
    let (store, object_state, mask_state, plain, paper) = (&store, &object_state, &mask_state, &plain, &paper);
    let paint_object = move |cx: &ContentPainterCx, rect: egui::Rect| {
        if !object_state.as_ref().is_some_and(|s| store.paint(cx, rect, s, plain))
            && let Some(c) = fill
        {
            cx.data_shapes([egui::Shape::rect_filled(rect.shrink(rect.width() * 0.2), 0.0, c)]);
        }
    };
    let paint_mask = move |cx: &ContentPainterCx, rect: egui::Rect| {
        if has_mask {
            if let Some(s) = mask_state {
                store.paint(cx, rect, s, paper);
            }
            if disabled {
                let red = craft::ui::style::DataColor::new(super::swatches::OFF_MARK);
                cx.with_draw(|draw| {
                    for line in [[rect.left_top(), rect.right_bottom()], [rect.right_top(), rect.left_bottom()]] {
                        draw.stroke(&PaintShape::Path(line.to_vec()), StrokeWidth::Thick, red);
                    }
                });
            }
        } else {
            cx.icon(rect, &IconSpec::svg(IconId::new("dc-mask-none")));
        }
    };
    // The list asks for its row in each of its runs.
    let row = || {
        let row = ListRow::new(tl!("Opacity Mask")).thumbnails_only().thumb(ThumbSize::Large, paint_object).extra_thumb(paint_mask);
        let row = match mask {
            Some(m) => row.thumb_link(m.linked, if m.linked { tl!("Unlink the mask") } else { tl!("Link the mask") }),
            None => row,
        };
        row.thumb_target(if editing { ThumbTarget::Extra(0) } else { ThumbTarget::First })
    };
    let keys = [()];
    List::<()>::new(tl!("Opacity mask thumbnails"))
        .select(SelectMode::None)
        .natural_height(true)
        .enabled(object)
        .show_items(ui, &keyed(&keys, |_| ()), &mut |_, _| row())
        .events
}

/// The Transparency panel: blend mode and opacity of the selection, or of the Appearance panel's active fill/stroke (whose object-only controls are greyed out), the opacity mask and the knockout options.
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    // While editing a mask the panel shows the masked object (the selection is its mask art).
    let editing = app.session.active().is_some_and(|d| d.doc.mask_edit.is_some());
    let (info, n) = state(app);
    let has = n.is_some();
    let (isolate, knockout_shape) = (info.isolate.unwrap_or(false), info.knockout_shape.unwrap_or(false));
    // Opacity and blend of the Appearance panel's active item (not while a mask is edited: the
    // panel then shows the masked object), else of the targets, blank where they differ; with no
    // target, the defaults for new art.
    let item = (!editing).then(|| app.session.appearance_item()).flatten().and_then(|_| current_transparency(app));
    let object = object_controls(app, editing);
    let (op, blend) = match item {
        Some((o, b)) => (Some(o), Some(b)),
        None if has => (info.opacity.map(|o| (o / 100.0) as f32), info.blend),
        None => (Some(1.0), Some(vectorcraft_color::BlendMode::Normal)),
    };
    let (mut shown_blend, blend_before) = (blend.map_or(Mixed::Mixed, Mixed::Same), blend.map_or(Mixed::Mixed, Mixed::Same));
    let entries = blend_entries();
    let mut opacity = None;
    Row::new().show(ui, |ui, _| {
        Select::new(tl!("Blend Mode"), Bind::bound_mixed(&mut shown_blend), &entries).enabled(has).show(ui);
        opacity = Some(
            NumberField::new(tl!("Opacity"), Bind::given(op.map(|o| f64::from(o) * 100.0)))
                .unit_kind(UnitKind::Percent)
                .range(0.0..=100.0)
                .popup_slider(0.0..=100.0)
                .enabled(has)
                .show(ui)
                .edit,
        );
    });
    if let (Mixed::Same(b), true) = (&shown_blend, shown_blend != blend_before) {
        app.run("transparency.set", json!({"blend": b.label()})).ok();
    }
    if let Some(opacity) = opacity {
        apply_edits(app, &opacity, EditPath::Preview, |app, v: &Option<f64>, phase| {
            if let Some(percent) = v {
                live_run(app, "Opacity", "transparency.set", json!({ "opacity": percent }), phase);
            }
        });
    }
    Divider::horizontal().show(ui);

    let mask = n.as_deref().and_then(|n| n.extras().mask.as_deref());
    let (new_clip, new_invert) = app.session.new_mask_defaults();
    let (mut clip, mut invert) = mask.map_or((new_clip, new_invert), |m| (m.clip, m.invert));
    let (clip_before, invert_before) = (clip, invert);
    // Make Mask is off in isolation mode and pattern editing mode for one object, which it would take into mask editing.
    let can_make = has && !editing && crate::menus::enabled(app, "transparency.makeOpacityMask");
    let (hide_thumbs, hide_options) = (app.ui.transparency_panel.hide_thumbs, app.ui.transparency_panel.hide_options);
    let mut cmds: Vec<(&'static str, Value)> = vec![];
    let mut events = vec![];
    Row::new().show(ui, |ui, _| {
        if !hide_thumbs {
            events = mask_row(app, ui, n.as_ref(), mask, editing, object);
        }
        Stack::new().show(ui, |ui, _| {
            let label = if mask.is_some() { tl!("Release") } else { tl!("Make Mask") };
            let enabled = object && (mask.is_some() || can_make);
            let button = Button::new(label).enabled(enabled).tooltip(tl!("Select the art (and, on top of it, the mask object)")).show(ui);
            if button.clicked() {
                cmds.push((if mask.is_some() { "transparency.releaseOpacityMask" } else { "transparency.makeOpacityMask" }, json!({})));
            }
            let with_mask = object && mask.is_some();
            Checkbox::new(Bind::bound(&mut clip)).label(tl!("Clip")).enabled(with_mask).show(ui);
            Checkbox::new(Bind::bound(&mut invert)).label(tl!("Invert Mask")).enabled(with_mask).show(ui);
        });
    });
    if clip != clip_before {
        cmds.push(("transparency.setOpacityMask", json!({"clip": clip})));
    }
    if invert != invert_before {
        cmds.push(("transparency.setOpacityMask", json!({"invert": invert})));
    }
    for event in events {
        match (event, mask, &n) {
            (CollectionEvent::Item(_, RowEvent::ThumbClicked { target: ThumbTarget::First, .. }), _, _) if editing => {
                cmds.push(("transparency.stopEditingOpacityMask", json!({})));
            }
            (CollectionEvent::Item(_, RowEvent::ThumbClicked { target: ThumbTarget::Extra(_), mods, .. }), Some(m), Some(n)) => {
                cmds.extend(mask_click(m, n.id, mods, editing));
            }
            (CollectionEvent::Item(_, RowEvent::LinkToggled { .. }), Some(m), _) => {
                cmds.push(("transparency.setOpacityMask", json!({"linked": !m.linked})));
            }
            _ => {}
        }
    }

    if !hide_options {
        Divider::horizontal().show(ui);
        let enabled = has && object;
        let (mut isolated, mut shape) = (isolate, knockout_shape);
        // Neutral (and mixed) show a dash; a click moves on → neutral → off → on (mixed → on).
        let mut group = match info.knockout {
            Some(Knockout::On) => Mixed::Same(true),
            Some(Knockout::Off) => Mixed::Same(false),
            _ => Mixed::Mixed,
        };
        let group_before = group;
        Checkbox::new(Bind::bound(&mut isolated)).label(tl!("Isolate Blending")).enabled(enabled).show(ui);
        Checkbox::new(Bind::bound_mixed(&mut group)).label(tl!("Knockout Group")).enabled(enabled).show(ui);
        Checkbox::new(Bind::bound(&mut shape)).label(tl!("Opacity & Mask Define Knockout Shape")).enabled(enabled).show(ui);
        if isolated != isolate {
            cmds.push(("transparency.set", json!({"isolate": isolated})));
        }
        if group != group_before {
            cmds.push(("transparency.set", json!({"knockout": info.knockout.map_or(Knockout::On, Knockout::cycle).label()})));
        }
        if shape != knockout_shape {
            cmds.push(("transparency.set", json!({"knockoutShape": shape})));
        }
    }
    if !has && selection_len(app) == 0 {
        Text::new(tl!("No Selection")).shade(Shade::Muted).show(ui);
    }
    for (id, params) in cmds {
        app.run(id, params).ok();
    }
}

/// The menu's opacity-mask rows (label, command, enabled) for the target's `mask`; Make needs a
/// target without one, outside mask editing (`can_make`).
fn mask_items(mask: Option<&OpacityMask>, can_make: bool) -> [(&'static str, &'static str, bool); 4] {
    match mask {
        Some(m) => [
            (tl!("Make Opacity Mask"), "transparency.makeOpacityMask", false),
            (tl!("Release Opacity Mask"), "transparency.releaseOpacityMask", true),
            if m.disabled {
                (tl!("Enable Opacity Mask"), "transparency.enableOpacityMask", true)
            } else {
                (tl!("Disable Opacity Mask"), "transparency.disableOpacityMask", true)
            },
            if m.linked {
                (tl!("Unlink Opacity Mask"), "transparency.unlinkOpacityMask", true)
            } else {
                (tl!("Link Opacity Mask"), "transparency.linkOpacityMask", true)
            },
        ],
        None => [
            (tl!("Make Opacity Mask"), "transparency.makeOpacityMask", can_make),
            (tl!("Release Opacity Mask"), "transparency.releaseOpacityMask", false),
            (tl!("Disable Opacity Mask"), "transparency.disableOpacityMask", false),
            (tl!("Unlink Opacity Mask"), "transparency.unlinkOpacityMask", false),
        ],
    }
}

/// The masked object's mask, also while editing it (the selection then is the mask art), whether the
/// object controls apply, and whether Make is possible.
fn menu_context(app: &VectorcraftApp) -> (Option<Arc<Node>>, bool, bool) {
    let editing = app.session.active().is_some_and(|d| d.doc.mask_edit.is_some());
    let n = first_node(app, &app.session.transparency_info());
    let can_make = n.is_some() && !editing && crate::menus::enabled(app, "transparency.makeOpacityMask");
    (n, object_controls(app, editing), can_make)
}

/// The panel menu's rows: the two view toggles, the opacity-mask commands, the new-mask defaults and the page options.
pub(crate) fn menu_rows(app: &VectorcraftApp) -> Vec<PanelRow> {
    let command = |id: &str| PanelRow::Command(CommandRef::new(id));
    let (n, _, can_make) = menu_context(app);
    let items = mask_items(n.as_ref().and_then(|n| n.extras().mask.as_deref()), can_make);
    let mut rows = vec![command(THUMBNAILS), command(OPTIONS), PanelRow::Separator];
    rows.extend(items.iter().map(|(_, id, _)| command(id)));
    rows.extend([
        PanelRow::Separator,
        command("transparency.toggleNewMasksClipping"),
        command("transparency.toggleNewMasksInverted"),
        PanelRow::Separator,
        command("transparency.togglePageIsolatedBlending"),
        command("transparency.togglePageKnockoutGroup"),
    ]);
    rows
}

/// The titles, enabled and checked states of the menu's commands.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    let panel = &app.ui.transparency_panel;
    states.set(THUMBNAILS, CommandState::new().title(if panel.hide_thumbs { tl!("Show Thumbnails") } else { tl!("Hide Thumbnails") }));
    states.set(OPTIONS, CommandState::new().title(if panel.hide_options { tl!("Show Options") } else { tl!("Hide Options") }));
    let (n, object, can_make) = menu_context(app);
    for (label, id, enabled) in mask_items(n.as_ref().and_then(|n| n.extras().mask.as_deref()), can_make) {
        states.set(id, CommandState::new().title(label).enabled(enabled && object));
    }
    let (clip, invert) = app.session.new_mask_defaults();
    states.set("transparency.toggleNewMasksClipping", CommandState::new().checked(clip));
    states.set("transparency.toggleNewMasksInverted", CommandState::new().checked(invert));
    let page = app.session.active().map(|d| (d.doc.page_isolate, d.doc.page_knockout));
    let (isolate, knockout) = page.unwrap_or_default();
    states.set("transparency.togglePageIsolatedBlending", CommandState::new().enabled(page.is_some()).checked(isolate));
    states.set("transparency.togglePageKnockoutGroup", CommandState::new().enabled(page.is_some()).checked(knockout));
}

/// Runs the panel's own `ui.transparency.*` commands; `None` for any other id.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    let panel = &mut app.ui.transparency_panel;
    match id {
        THUMBNAILS => panel.hide_thumbs = !panel.hide_thumbs,
        OPTIONS => panel.hide_options = !panel.hide_options,
        _ => return None,
    }
    Some(Ok(json!({})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::panel_harness::Panel;
    use vectorcraft_engine::Session;

    fn run_cmd(app: &mut VectorcraftApp, id: &str, p: serde_json::Value) -> serde_json::Value {
        app.session.execute(id, &p).unwrap()
    }

    fn new_app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        run_cmd(&mut app, "file.new", json!({"width": 100, "height": 100}));
        app
    }

    #[test]
    fn panel_in_mask_editing_mode_shows_and_edits_the_masked_object() {
        let mut app = new_app();
        let a = run_cmd(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].as_u64().unwrap();
        run_cmd(&mut app, "transparency.set", json!({"opacity": 60}));
        // One object: an empty mask, drawn in editing mode.
        run_cmd(&mut app, "transparency.makeOpacityMask", json!({}));
        run_cmd(&mut app, "shape.ellipse", json!({"x": 0, "y": 0, "width": 30, "height": 30}));
        let mut panel = Panel::new(app, show);
        assert_eq!(panel.value("Opacity").as_deref(), Some("60%"), "the masked object's opacity, not the mask art's");
        assert!(panel.shows("Release"));
        let (info, n) = state(&mut panel.app());
        assert_eq!((info.ids, n.as_ref().map(|n| n.id.0)), (vec![vectorcraft_doc::NodeId(a)], Some(a)));
        // The menu is enabled for the masked object while editing (Make is not).
        let items = mask_items(n.as_ref().and_then(|n| n.extras().mask.as_deref()), false);
        assert_eq!(items.map(|(_, _, on)| on), [false, true, true, true]);
        // The panel's controls run without ids and reach the masked object.
        for (id, p) in [
            ("transparency.setOpacityMask", json!({"clip": false})),
            ("transparency.unlinkOpacityMask", json!({})),
            ("transparency.set", json!({"blend": "Multiply"})),
        ] {
            run_cmd(&mut panel.app(), id, p);
        }
        panel.frame();
        let app = panel.app();
        let st = app.session.active().unwrap();
        let obj = st.doc.node(vectorcraft_doc::NodeId(a)).unwrap();
        let m = obj.extras().mask.as_ref().unwrap();
        assert!(!m.clip && !m.linked && obj.blend == vectorcraft_color::BlendMode::Multiply);
        assert!(st.doc.mask_edit.is_some());
        drop(app);
        // Release leaves editing and puts the drawn mask art back.
        run_cmd(&mut panel.app(), "transparency.releaseOpacityMask", json!({}));
        panel.frame();
        assert!(panel.shows("Make Mask"));
        assert!(panel.app().session.active().unwrap().doc.mask_edit.is_none());
    }

    /// Picking other gradient mesh points (which gives the document no new revision) shows their
    /// opacity, the one `transparency.set` changes then.
    #[test]
    fn picking_mesh_points_shows_their_opacity() {
        use vectorcraft_engine::ViewInfo;
        use vectorcraft_tools::{PointerEvent, PointerKind};
        let mut app = new_app();
        let a = run_cmd(&mut app, "shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50}))["id"].clone();
        run_cmd(&mut app, "object.mesh.create", json!({"rows": 2, "cols": 2}));
        run_cmd(&mut app, "object.mesh.setPointColor", json!({"id": a, "index": 0, "opacity": 0.3}));
        run_cmd(&mut app, "paint.setStroke", json!({"color": "#000000"}));
        let v = ViewInfo::default();
        app.session.select_tool("mesh", v).unwrap();
        let click = |app: &mut VectorcraftApp, x: f64, y: f64| {
            for k in [PointerKind::Down, PointerKind::Up] {
                app.session.pointer(&PointerEvent::new(k, x, y), v).unwrap();
            }
        };
        let picked = |app: &VectorcraftApp| app.session.mesh_points_picked().map(|p| p.into_iter().flat_map(|(_, v)| v).collect::<Vec<_>>());
        click(&mut app, 35.0, 10.0);
        assert_ne!(picked(&app), Some(vec![0]), "the click picks the top middle point");
        assert_eq!(picked(&app).map(|v| v.len()), Some(1));
        assert_eq!(state(&mut app).0.opacity, Some(100.0));
        click(&mut app, 10.0, 10.0);
        assert_eq!(picked(&app), Some(vec![0]), "the click picks the top-left point");
        assert_eq!(state(&mut app).0.opacity, Some(30.0), "the newly picked point's opacity");
        // An active Appearance row takes the opacity, so the values are the object's again.
        let rows = app
            .session
            .active()
            .unwrap()
            .selection
            .objects
            .first()
            .map(|id| app.session.active().unwrap().doc.node(*id).unwrap().appearance.items.len());
        assert!(rows.is_some_and(|n| n > 0), "the mesh has its stroke row");
        run_cmd(&mut app, "appearance.setActiveItem", json!({"index": 0}));
        assert_eq!(picked(&app), None);
        assert_eq!(app.session.transparency_info().opacity, Some(100.0), "the object's opacity, not the point's");
        assert_eq!(run_cmd(&mut app, "transparency.info", json!({}))["opacity"], json!(100.0));
    }

    #[test]
    fn mixed_values_show_blank() {
        let mut app = new_app();
        let a = run_cmd(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].clone();
        run_cmd(&mut app, "transparency.set", json!({"opacity": 50, "blend": "Screen"}));
        let b = run_cmd(&mut app, "shape.rectangle", json!({"x": 60, "y": 0, "width": 30, "height": 30}))["id"].clone();
        run_cmd(&mut app, "select.set", json!({"ids": [a, b]}));
        let mut panel = Panel::new(app, show);
        let blank = |v: Option<String>| v.is_none_or(|v| v.is_empty());
        assert!(blank(panel.value("Opacity")) && blank(panel.value("Blend Mode")));
        run_cmd(&mut panel.app(), "select.set", json!({"ids": [a]}));
        panel.frame();
        assert_eq!((panel.value("Opacity").as_deref(), panel.value("Blend Mode").as_deref()), (Some("50%"), Some("Screen")));
        // Nothing selected: the defaults for new art, disabled.
        run_cmd(&mut panel.app(), "select.none", json!({}));
        panel.frame();
        assert_eq!(panel.value("Opacity").as_deref(), Some("100%"), "a percent field reads its number and its unit");
        assert!(panel.shows("No Selection") && !panel.enabled("Opacity"));
    }

    /// Each knockout state and the state a click on the box moves it to.
    #[test]
    fn knockout_states_and_page_group_items() {
        for (start, next) in [("on", "neutral"), ("neutral", "off"), ("off", "on")] {
            let mut app = new_app();
            run_cmd(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}));
            run_cmd(&mut app, "transparency.set", json!({"knockout": start}));
            let mut panel = Panel::new(app, show);
            assert!(panel.shows("Opacity & Mask Define Knockout Shape"));
            panel.click("Knockout Group");
            let after = panel.app().session.transparency_info().knockout.map(|k| k.label().to_lowercase());
            assert_eq!(after.as_deref(), Some(next), "from {start}");
        }
        let mut app = new_app();
        run_cmd(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}));
        let page = |app: &VectorcraftApp| {
            let mut states = CommandStates::new();
            menu_states(app, &mut states);
            ["transparency.togglePageKnockoutGroup", "transparency.togglePageIsolatedBlending"]
                .map(|id| states.get(&CommandRef::new(id)).and_then(CommandState::is_checked) == Some(true))
        };
        assert_eq!(page(&app), [false, false]);
        run_cmd(&mut app, "transparency.togglePageKnockoutGroup", json!({}));
        run_cmd(&mut app, "transparency.togglePageIsolatedBlending", json!({}));
        assert_eq!(page(&app), [true, true]);
    }

    /// The panel's state holds the target as the document holds it, a new one after an edit and
    /// after Undo; the object thumbnail of a masked object draws as the object without its mask.
    #[test]
    fn state_holds_the_documents_node_and_the_object_thumbnail_leaves_out_the_mask() {
        let mut app = new_app();
        let a = run_cmd(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].clone();
        let b = run_cmd(&mut app, "shape.ellipse", json!({"x": 10, "y": 10, "width": 20, "height": 20}))["id"].clone();
        run_cmd(&mut app, "select.set", json!({"ids": [a, b]}));
        run_cmd(&mut app, "transparency.makeOpacityMask", json!({}));
        let held = |app: &VectorcraftApp, id: NodeId| super::super::layers::node_arc(&app.session.active().unwrap().doc, id).expect("held");
        let n = state(&mut app).1.expect("the masked object");
        assert!(n.extras().mask.is_some(), "the object has its mask");
        assert!(Arc::ptr_eq(&n, &held(&app, n.id)), "the node the document holds");
        assert!(Arc::ptr_eq(&state(&mut app).1.expect("cached"), &n), "the cached state gives the same node");
        // The object thumbnail: as a copy with the mask removed draws, not as the masked object.
        let mut bare = (*n).clone();
        bare.extras_mut().mask = None;
        let doc = app.session.active().unwrap().doc.clone();
        let without = thumb_image(&doc, &n, false, 44, None).expect("the object draws");
        assert!(without == thumb_image(&doc, &bare, false, 44, None).expect("the bare object draws"), "the object without its mask");
        assert!(without != thumb_image(&doc, &n, true, 44, None).expect("the mask draws"), "the mask changes the drawing");
        // An edit and its Undo: the state follows the document.
        run_cmd(&mut app, "transparency.set", json!({"opacity": 40}));
        let (info, edited) = state(&mut app);
        let edited = edited.expect("the edited object");
        assert_eq!((info.opacity, edited.opacity), (Some(40.0), 0.4));
        assert!(Arc::ptr_eq(&edited, &held(&app, n.id)) && !Arc::ptr_eq(&edited, &n), "the edited node");
        run_cmd(&mut app, "edit.undo", json!({}));
        let (info, undone) = state(&mut app);
        let undone = undone.expect("the object after Undo");
        assert_eq!((info.opacity, undone.opacity), (Some(100.0), 1.0));
        assert!(Arc::ptr_eq(&undone, &held(&app, n.id)), "the node Undo put back");
    }
}
