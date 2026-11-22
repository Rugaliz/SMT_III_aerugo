//! Turning the visible part of a [`UiTree`] into textured quads.

use crate::color::{Color, Color32};
use crate::math::Affine;
use crate::prefab::TextureRef;
use crate::tree::{Image, NodeId, UiTree};

/// A text component to draw: the node, its rectangle in its local space, its transform to canvas units and
/// its scale relative to the canvas (the product of the local y scales below the canvas).
#[derive(Clone, Debug, PartialEq)]
pub struct UiText {
    pub node: NodeId,
    pub rect: [f32; 4],
    pub to_canvas: Affine,
    pub scale_ratio: f32,
    /// Alpha from the canvas groups above the text.
    pub alpha: f32,
}

/// One drawing step of the tree, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum UiDraw {
    Quad(UiQuad),
    Text(UiText),
}

/// The root canvas size in canvas units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Canvas {
    pub width: f32,
    pub height: f32,
}

impl Canvas {
    /// The size the original's screens are designed for.
    pub const DESIGN: Canvas = Canvas { width: 1920.0, height: 1080.0 };
}

/// One quad in canvas units (origin at the canvas center, y up). Corners are in triangle-strip order.
#[derive(Clone, Debug, PartialEq)]
pub struct UiQuad {
    /// `None`: untextured (a plain white texture).
    pub texture: Option<TextureRef>,
    pub pos: [[f32; 2]; 4],
    /// Texture coordinates with v up (row 0 of the stored texture is v = 0).
    pub uv: [[f32; 2]; 4],
    pub color: Color32,
}

impl UiTree {
    /// Every shown, enabled image and text, in drawing order (parents before children, siblings in order).
    pub fn draw_list(&self) -> Vec<UiDraw> {
        let mut out = Vec::new();
        self.visit(self.root(), Affine::IDENTITY, 1.0, 1.0, &mut out);
        out
    }

    /// Only the image quads of [`draw_list`](Self::draw_list).
    pub fn quads(&self) -> Vec<UiQuad> {
        self.draw_list()
            .into_iter()
            .filter_map(|d| match d {
                UiDraw::Quad(q) => Some(q),
                UiDraw::Text(_) => None,
            })
            .collect()
    }

    fn visit(&self, id: NodeId, parent: Affine, parent_scale: f32, parent_alpha: f32, out: &mut Vec<UiDraw>) {
        let n = self.node(id);
        if !n.active_self {
            return;
        }
        // Canvas groups multiply the alpha of everything below them (one ignoring its parents starts over).
        let alpha = match &n.canvas_group {
            Some(g) if g.enabled && g.ignore_parent_groups => g.alpha,
            Some(g) if g.enabled => parent_alpha * g.alpha,
            _ => parent_alpha,
        };
        let (rect, to_parent) = self.layout(id);
        let m = parent.mul(&to_parent);
        // The root is the canvas: its own scale is the canvas scale, not part of the ratio.
        let scale = if id == self.root() { 1.0 } else { parent_scale * n.transform.local_scale[1] };
        if let Some(img) = &n.image
            && img.enabled
            && let Some(mut q) = self.image_quad(img, rect, n.transform.pivot, &m)
        {
            if alpha != 1.0 {
                q.color =
                    Color32::from(Color::new(img.color.r, img.color.g, img.color.b, img.color.a * alpha));
            }
            out.push(UiDraw::Quad(q));
        }
        if let Some(t) = &n.text
            && t.enabled
            && !t.text.is_empty()
        {
            out.push(UiDraw::Text(UiText { node: id, rect, to_canvas: m, scale_ratio: scale, alpha }));
        }
        for &c in &n.children {
            self.visit(c, m, scale, alpha, out);
        }
    }

    /// Mesh of a simple image: the node rectangle (shrunk by the sprite's transparent padding, optionally
    /// fitted to the sprite's aspect ratio) textured with the sprite's stored rectangle.
    fn image_quad(&self, img: &Image, rect: [f32; 4], pivot: [f32; 2], m: &Affine) -> Option<UiQuad> {
        let color = Color32::from(img.color);
        let corner = |x: f32, y: f32| {
            let p = m.apply([x, y, 0.0]);
            [p[0], p[1]]
        };
        let Some(si) = img.sprite else {
            // No sprite: the plain rectangle, texture coordinates 0..1.
            let [x, y, w, h] = rect;
            return Some(UiQuad {
                texture: None,
                pos: [corner(x, y), corner(x, y + h), corner(x + w, y), corner(x + w, y + h)],
                uv: [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]],
                color,
            });
        };
        if img.kind != 0 || img.use_sprite_mesh {
            static REPORTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !REPORTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                log::warn!("image kind {} (sprite mesh {}) is not drawn yet", img.kind, img.use_sprite_mesh);
            }
            return None;
        }
        let s = &self.sprites[si];
        let pad = s.padding();
        let size = [s.rect[2], s.rect[3]];
        let (sw, sh) = (size[0].round_ties_even(), size[1].round_ties_even());
        let v = [pad[0] / sw, pad[1] / sh, (sw - pad[2]) / sw, (sh - pad[3]) / sh];
        let mut r = rect;
        if img.preserve_aspect && size[0] * size[0] + size[1] * size[1] > 0.0 {
            preserve_aspect(&mut r, size, pivot);
        }
        let d = [r[0] + r[2] * v[0], r[1] + r[3] * v[1], r[0] + r[2] * v[2], r[1] + r[3] * v[3]];
        let uv = s.outer_uv();
        Some(UiQuad {
            texture: Some(s.texture.clone()),
            pos: [corner(d[0], d[1]), corner(d[0], d[3]), corner(d[2], d[1]), corner(d[2], d[3])],
            uv: [[uv[0], uv[1]], [uv[0], uv[3]], [uv[2], uv[1]], [uv[2], uv[3]]],
            color,
        })
    }
}

/// Shrink `r` to the sprite's aspect ratio, keeping the pivot point in place.
fn preserve_aspect(r: &mut [f32; 4], sprite_size: [f32; 2], pivot: [f32; 2]) {
    let ratio = sprite_size[0] / sprite_size[1];
    if r[2] / r[3] < ratio {
        let old_h = r[3];
        r[3] = r[2] * (1.0 / ratio);
        r[1] += (old_h - r[3]) * pivot[1];
    } else {
        let old_w = r[2];
        r[2] = r[3] * ratio;
        r[0] += (old_w - r[2]) * pivot[0];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::prefab::{Prefab, PrefabNode};
    use crate::tree::{RectTransform, Sprite};
    use aerugo_unity::ObjRef;

    fn logo_prefab() -> Prefab {
        let rt = RectTransform { size_delta: [1920.0, 1080.0], ..Default::default() };
        let mut root = PrefabNode::new("logo_root", None, vec![1], rt);
        root.active = true;
        let mut img = PrefabNode::new("logo", Some(0), vec![], rt);
        img.active = false;
        img.image = Some(Image {
            enabled: true,
            color: Color::WHITE,
            sprite: Some(0),
            kind: 0,
            preserve_aspect: false,
            use_sprite_mesh: false,
        });
        let sprite = Sprite {
            texture: TextureRef { bundle: "b".into(), obj: ObjRef { node: 0, path_id: 7 } },
            texture_size: [1920.0, 1080.0],
            rect: [0.0, 0.0, 1920.0, 1080.0],
            texture_rect: [0.0, 0.0, 1920.0, 1080.0],
            texture_rect_offset: [0.0, 0.0],
        };
        Prefab { nodes: vec![root, img], sprites: vec![sprite] }
    }

    #[test]
    fn full_screen_logo_covers_the_canvas() {
        let mut ui = UiTree::new("canvas", Canvas::DESIGN);
        let root = ui.instantiate(&logo_prefab(), ui.root());
        assert!(ui.quads().is_empty(), "the logo starts hidden");
        let logo = ui.find(root, "logo").unwrap();
        ui.set_active(logo, true);
        ui.set_image_color(logo, Color::new(255.0, 255.0, 255.0, 0.5));
        let q = ui.quads();
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].pos, [[-960.0, -540.0], [-960.0, 540.0], [960.0, -540.0], [960.0, 540.0]]);
        assert_eq!(q[0].uv, [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]]);
        assert_eq!(q[0].color, Color32([255, 255, 255, 128]));
    }

    #[test]
    fn aspect_is_preserved_around_the_pivot() {
        let mut r = [-100.0, -100.0, 200.0, 200.0];
        preserve_aspect(&mut r, [2.0, 1.0], [0.5, 0.5]);
        assert_eq!(r, [-100.0, -50.0, 200.0, 100.0]);
        let mut r = [0.0, 0.0, 200.0, 100.0];
        preserve_aspect(&mut r, [1.0, 1.0], [0.0, 0.0]);
        assert_eq!(r, [0.0, 0.0, 100.0, 100.0]);
    }
}
