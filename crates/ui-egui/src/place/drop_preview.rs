//! Read-only PDF drag previews. At most one read runs at a time, off the UI thread; switching
//! files keeps only the latest request, and leaving the canvas discards its image, not an edit.

use std::sync::mpsc::{Receiver, TryRecvError, channel};

use egui::{Pos2, Rect};
use serde_json::json;
use vectorcraft_engine::cmd::{fileio, place};
use vectorcraft_engine::file_access::ScopeHandle;
use vectorcraft_geom::Point;

use super::{Thumb, Tokens, VectorcraftApp};

const PIXELS: u32 = 256;

struct Image {
    thumb: Thumb,
    width: f64,
    height: f64,
}

type Read = Receiver<Option<Image>>;

#[derive(Default)]
pub(super) struct Preview {
    path: Option<String>,
    reading: Option<(String, Read)>,
    image: Option<Image>,
    failed: bool,
}

impl Preview {
    pub(super) fn forget_texture(&mut self) {
        if let Some(image) = &mut self.image {
            image.thumb.tex = None;
        }
    }

    fn complete(&mut self, path: &str, image: Option<Image>) {
        if self.path.as_deref() == Some(path) {
            self.failed = image.is_none();
            self.image = image;
        }
    }

    fn update(&mut self, path: Option<String>, ctx: &egui::Context) {
        if self.path != path {
            self.path = path;
            self.image = None;
            self.failed = false;
        }
        if let Some((name, read)) = self.reading.take() {
            match read.try_recv() {
                Ok(image) => self.complete(&name, image),
                Err(TryRecvError::Empty) => self.reading = Some((name, read)),
                Err(TryRecvError::Disconnected) => self.complete(&name, None),
            }
        }
        if self.reading.is_some() || self.image.is_some() || self.failed {
            return;
        }
        let Some(path) = self.path.clone() else { return };
        let (tx, read) = channel();
        let name = path.clone();
        let scope = ScopeHandle::capture();
        let repaint = ctx.clone();
        let started = std::thread::Builder::new().name("vectorcraft-drop-preview".into()).spawn(move || {
            // A preview is optional. An unreadable, encrypted or malformed PDF keeps the canvas
            // highlight; dropping still uses the normal placement path to report its own error.
            let image = scope.run(|| vectorcraft_engine::guard::catch_panic(|| read_image(&path))).ok().flatten();
            // The app may have closed while reading; the preview has no document side effects.
            let _ = tx.send(image);
            repaint.request_repaint();
        });
        if started.is_ok() {
            self.reading = Some((name, read));
        } else {
            self.failed = true;
        }
    }

    #[cfg(test)]
    pub(super) fn wait(&mut self) {
        if let Some((path, read)) = self.reading.take() {
            let image = read.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
            self.complete(&path, image);
        }
    }
}

fn read_image(path: &str) -> Option<Image> {
    let info = place::read_info(&json!({"path": path, "page": 1, "thumbnail": PIXELS})).ok()?;
    let (width, height) = (info["width"].as_f64()?, info["height"].as_f64()?);
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return None;
    }
    let png = vectorcraft_format::base64_decode(info["thumbnailBase64"].as_str()?)?;
    let decoded = craft_image_codecs::decode_as(craft_image_codecs::Format::Png, &png).ok()?;
    if decoded.width() > PIXELS || decoded.height() > PIXELS {
        return None;
    }
    let color = egui::ColorImage::from_rgba_unmultiplied([decoded.width() as usize, decoded.height() as usize], &decoded.to_rgba8());
    Some(Image { thumb: Thumb { image: std::sync::Arc::new(color), tex: None }, width, height })
}

pub(super) fn paint(app: &mut VectorcraftApp, ctx: &egui::Context, painter: &egui::Painter, rect: Rect, pos: Option<Pos2>) {
    let path = pos.filter(|p| rect.contains(*p)).and_then(|_| {
        ctx.input(|i| {
            i.raw.hovered_files.iter().filter_map(|f| f.path.as_ref()).find_map(|p| {
                let name = p.to_string_lossy();
                (fileio::extension(&name) == "pdf" && app.drop_target(&name, pos, i.modifiers.shift) != super::DropTarget::Open)
                    .then(|| name.into_owned())
            })
        })
    });
    let xf = app.view().map(|v| crate::canvas::Xf::at(rect, v, app.canvas_ui_zoom));
    app.place.drop_preview.update(path, ctx);
    let preview = &mut app.place.drop_preview;
    if preview.path.is_none() {
        return;
    }
    // Windows OLE drags do not deliver CursorMoved. Poll only while a file is over the canvas.
    ctx.request_repaint_after(std::time::Duration::from_millis(16));
    let (Some(pos), Some(xf), Some(image)) = (pos, xf, preview.image.as_mut()) else { return };
    let center = xf.to_doc(pos);
    let corners = [
        Point::new(center.x - image.width / 2.0, center.y - image.height / 2.0),
        Point::new(center.x + image.width / 2.0, center.y - image.height / 2.0),
        Point::new(center.x + image.width / 2.0, center.y + image.height / 2.0),
        Point::new(center.x - image.width / 2.0, center.y + image.height / 2.0),
    ]
    .map(|p| xf.to_screen(p));
    if !corners.iter().all(|p| p.is_finite()) {
        return;
    }
    let texture =
        image.thumb.tex.get_or_insert_with(|| ctx.load_texture("pdf-drop-preview", image.thumb.image.clone(), egui::TextureOptions::LINEAR));
    let mut mesh = egui::Mesh::with_texture(texture.id());
    for (pos, uv) in corners.into_iter().zip([egui::pos2(0.0, 0.0), egui::pos2(1.0, 0.0), egui::pos2(1.0, 1.0), egui::pos2(0.0, 1.0)]) {
        mesh.vertices.push(egui::epaint::Vertex { pos, uv, color: egui::Color32::from_white_alpha(150) });
    }
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    painter.add(egui::Shape::mesh(mesh));
    painter.add(egui::Shape::closed_line(corners.to_vec(), egui::Stroke::new(1.0, Tokens::get(ctx).accent)));
}
