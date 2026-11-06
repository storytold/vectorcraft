//! The objects' URLs (Window › Attributes) as link annotations: a rectangle over each linked
//! object's bounds on the page that opens its URL (ISO 32000-1 §12.5.6.5 Link Annotations, with a
//! URI action, §12.6.4.7).

use super::{Affine, Exporter, Node, NodeKind, Rect, krilla};

/// Most links written on one page: the rest are left out with a warning.
pub(super) const MAX_LINKS: usize = 10_000;

/// The warning for the links left out past [`MAX_LINKS`].
pub(super) const TOO_MANY_LINKS: &str = "a page has more linked objects than a PDF page keeps links for: the rest aren't clickable";

/// Does a link open `uri`? Web and mail URLs only, the ones the app's Browser button opens: a URL
/// can come from an imported file, and a `javascript:` or `file:` one has no place in a PDF.
fn opens(uri: &str) -> bool {
    let scheme = uri.trim().split_once(':').map_or("", |(s, _)| s);
    ["http", "https", "mailto"].iter().any(|k| scheme.eq_ignore_ascii_case(k))
}

/// A link to write: the URI it opens, and the area it covers in page space (y down).
pub(super) struct Link {
    pub uri: String,
    pub rect: Rect,
}

impl Exporter<'_> {
    /// The links of the art drawn on a page: each linked object that shows (on a layer that
    /// prints, or any layer when non-printing layers are written; not on a template layer), over
    /// its bounds within the clipping groups it is in and within `area` (document space, the art
    /// drawn), mapped onto the page by `to_page`. A group's link comes before its objects', so an
    /// object's own link lies over it. The second value: links were left out past [`MAX_LINKS`].
    pub(super) fn links(&self, area: Rect, to_page: Affine) -> (Vec<Link>, bool) {
        let mut links = vec![];
        let mut more = false;
        for layer in &self.doc.layers {
            self.link_node(layer, area, to_page, &mut links, &mut more);
        }
        (links, more)
    }

    fn link_node(&self, n: &Node, within: Rect, to_page: Affine, links: &mut Vec<Link>, more: &mut bool) {
        if !n.visible || matches!(n.kind, NodeKind::Path { guide: true, .. }) {
            return;
        }
        if let NodeKind::Layer { template, printable, .. } = n.kind
            && (template || !(printable || self.non_printing))
        {
            return;
        }
        let bounds = n.visual_bounds_with(&self.symbols).filter(|b| [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite()));
        if let (Some(uri), Some(b)) = (n.url().filter(|u| opens(u)), bounds) {
            let shown = b.intersect(within);
            if shown.width() > 0.0 && shown.height() > 0.0 {
                let rect = to_page.transform_rect_bbox(shown);
                if [rect.x0, rect.y0, rect.x1, rect.y1].iter().any(|v| !v.is_finite()) {
                    // Off any page a reader can show: no link.
                } else if links.len() < MAX_LINKS {
                    links.push(Link { uri: uri_bytes(uri), rect });
                } else {
                    *more = true;
                }
            }
        }
        // What a clipping group holds shows only inside its clipping path's bounds.
        let within = match bounds {
            Some(b) if n.clips() => b.intersect(within),
            None if n.clips() => return,
            _ => within,
        };
        if within.width() <= 0.0 || within.height() <= 0.0 {
            return;
        }
        for child in n.children().into_iter().flatten() {
            self.link_node(child, within, to_page, links, more);
        }
    }
}

/// `uri` as a PDF URI (ISO 32000-1 §12.6.4.7: 7-bit ASCII): spaces, control characters and
/// non-ASCII bytes percent-encoded (RFC 3986 §2.1), leading and trailing spaces dropped.
fn uri_bytes(uri: &str) -> String {
    let mut out = String::with_capacity(uri.len());
    for b in uri.trim().bytes() {
        if b.is_ascii_graphic() {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The annotation of `link`: a borderless rectangle that opens its URI.
pub(super) fn annotation(link: Link) -> Option<krilla::annotation::Annotation> {
    let r = link.rect;
    let rect = krilla::geom::Rect::from_ltrb(r.x0 as f32, r.y0 as f32, r.x1 as f32, r.y1 as f32)?;
    let target = krilla::annotation::Target::Action(krilla::action::LinkAction::new(link.uri.clone()).into());
    Some(krilla::annotation::Annotation::new_link(krilla::annotation::LinkAnnotation::new(rect, target), Some(link.uri)))
}

#[cfg(test)]
mod tests {
    use super::{opens, uri_bytes};

    #[test]
    fn only_web_and_mail_urls_become_links() {
        for ok in ["https://example.com/a", "HTTP://example.com", "mailto:someone@example.com", " https://example.com "] {
            assert!(opens(ok), "{ok}");
        }
        for refused in ["javascript:alert(1)", "file:///etc/passwd", "smb://host/share", "ms-settings:", "example.com", ""] {
            assert!(!opens(refused), "{refused}");
        }
    }

    #[test]
    fn uris_are_written_in_ascii() {
        assert_eq!(uri_bytes(" https://example.com/a b "), "https://example.com/a%20b");
        assert_eq!(uri_bytes("https://example.com/café"), "https://example.com/caf%C3%A9");
        assert_eq!(uri_bytes("mailto:a@b.c?subject=x\ny"), "mailto:a@b.c?subject=x%0Ay");
    }
}
