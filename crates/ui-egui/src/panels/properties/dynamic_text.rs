//! Dynamic Text in the Properties panel's Paragraph section: the Dynamic Text checkbox for the
//! selected area type (Block, `type.dynamicText`) and, while it is on, its Word Gap and Line Gap
//! rows ([Dynamic Text overview](https://helpx.adobe.com/photoshop/desktop/text-typography/work-with-dynamic-text/dynamic-text-overview.html)
//! describes Block and its spacing between words and between lines).

use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_doc::{AreaOptions, NodeKind, TextKind};

use crate::VectorcraftApp;
use crate::widgets;

/// What the selected area type shows: whether Block is on in every frame (`None` when they
/// differ), the gaps every frame shares (`None` when they differ), and whether every frame can
/// take it (none is threaded).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Shown {
    pub(crate) on: Option<bool>,
    pub(crate) word_gap: Option<f64>,
    pub(crate) line_gap: Option<f64>,
    pub(crate) enabled: bool,
}

/// Dynamic Text of the selected area type (and area type in selected groups); `None` when the
/// selection holds no area type, or holds other type too (the command refuses it).
pub(crate) fn shown(app: &VectorcraftApp) -> Option<Shown> {
    let st = app.session.active()?;
    let threaded = |id| st.doc.text_threads.iter().any(|t| t.len() > 1 && t.contains(&id));
    let mut frames = vec![];
    let mut other = false;
    for n in st.selection.objects.iter().filter_map(|id| st.doc.node(*id)) {
        n.walk(&mut |c| {
            let NodeKind::Text(t) = &c.kind else { return };
            if matches!(t.kind, TextKind::Area { .. }) {
                let (word, line) = t.area.block_gaps();
                frames.push((t.dynamic_text, word, line, threaded(c.id)));
            } else {
                other = true;
            }
        });
    }
    let &(on, word, line, _) = frames.first().filter(|_| !other)?;
    Some(Shown {
        on: frames.iter().all(|f| f.0 == on).then_some(on),
        word_gap: frames.iter().all(|f| f.1 == word).then_some(word),
        line_gap: frames.iter().all(|f| f.2 == line).then_some(line),
        enabled: frames.iter().all(|f| !f.3),
    })
}

fn run(app: &mut VectorcraftApp, p: Value) {
    if let Err(e) = app.run("type.dynamicText", p) {
        app.status(e);
    }
}

/// The checkbox, and the gap rows while every selected frame has Block on. Nothing shows without
/// area type. Returns whether anything was drawn.
pub(crate) fn rows(app: &mut VectorcraftApp, ui: &mut Ui) -> bool {
    let Some(s) = shown(app) else { return false };
    if widgets::check(ui, tl!("Dynamic Text"), s.on == Some(true), s.enabled) {
        run(app, json!({ "on": s.on != Some(true) }));
    }
    if s.on != Some(true) {
        return true;
    }
    let fw = super::super::field_width(ui);
    let (lo, hi) = AreaOptions::WORD_GAP;
    ui.horizontal(|ui| {
        ui.label(tl!("Word Gap"));
        if let Some(v) = widgets::spin_plain_named(ui, "props-dt-word", Some(tl!("Word Gap")), s.word_gap, "%", 0, fw, 10.0, lo, &[]) {
            run(app, json!({ "wordGap": v.clamp(lo, hi) }));
        }
    });
    let (lo, hi) = AreaOptions::LINE_GAP;
    let unit = app.session.general_unit();
    ui.horizontal(|ui| {
        ui.label(tl!("Line Gap"));
        if let Some(v) = widgets::spin_field_named(ui, "props-dt-line", Some(tl!("Line Gap")), s.line_gap, unit, fw, 1.0, lo, &[]) {
            run(app, json!({ "lineGap": v.clamp(lo, hi) }));
        }
    });
    true
}
