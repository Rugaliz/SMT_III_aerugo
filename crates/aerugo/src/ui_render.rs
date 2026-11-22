//! Drawing the UI tree and the fade layer with the renderer.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use aerugo_gfx::{DrawItem, Gfx, Sprite, TextureId};
use aerugo_ui::{Canvas, Color, Color32, TextureRef, UiDraw, UiQuad, UiTree};
use anyhow::Result;

use crate::assets::Assets;
use crate::player_files::PlayerFiles;
use crate::text_render::{CanvasView, TextRenderer};

/// The part of the window the 1920x1080 canvas fills, as fractions of the window: x, y (from the top), w, h.
///
/// Like the original's cameras, the canvas keeps its 16:9 shape: wider windows get bars at the sides,
/// narrower ones bars at the top and bottom.
pub fn canvas_area(window: (u32, u32), canvas: Canvas) -> [f32; 4] {
    let r = (window.0 as f32 / window.1 as f32) / (canvas.width / canvas.height);
    if r < 1.0 { [0.0, (1.0 - r) * 0.5, 1.0, r] } else { [(1.0 - 1.0 / r) * 0.5, 0.0, 1.0 / r, 1.0] }
}

#[derive(Debug)]
pub struct UiRenderer {
    assets: Rc<RefCell<Assets>>,
    textures: HashMap<TextureRef, TextureId>,
    failed: HashSet<TextureRef>,
    white: Option<TextureId>,
    text: TextRenderer,
    /// The player's resource files, opened on first use (`None` inside: they failed to open).
    player: Option<Option<PlayerFiles>>,
}

impl UiRenderer {
    pub fn new(assets: Rc<RefCell<Assets>>) -> Self {
        Self {
            assets,
            textures: HashMap::new(),
            failed: HashSet::new(),
            white: None,
            text: TextRenderer::default(),
            player: None,
        }
    }

    fn white(&mut self, gfx: &mut Gfx) -> Result<TextureId> {
        if let Some(t) = self.white {
            return Ok(t);
        }
        let tex = aerugo_unity::Texture2D {
            name: "white".into(),
            width: 1,
            height: 1,
            format: aerugo_unity::TextureFormat::Rgba32,
            mip_count: 1,
            data: vec![255; 4],
        };
        let t = gfx.upload_texture(&tex)?;
        self.white = Some(t);
        Ok(t)
    }

    fn texture(&mut self, gfx: &mut Gfx, r: &TextureRef) -> Option<TextureId> {
        texture(&mut self.assets.borrow_mut(), &mut self.textures, &mut self.failed, gfx, r)
    }

    /// The UI tree's images and texts in order, then the fade layer (`[r, g, b, a]` with `a` in 0..=128) on
    /// top. `lang` is the language number (it picks the fonts).
    pub fn items(&mut self, gfx: &mut Gfx, ui: &UiTree, fade: [u8; 4], lang: usize) -> Result<Vec<DrawItem>> {
        let area = canvas_area(gfx.extent(), ui.canvas);
        let c = ui.canvas;
        let to_ndc = |[x, y]: [f32; 2]| {
            let fx = x / c.width + 0.5;
            let fy = 1.0 - (y / c.height + 0.5);
            [(area[0] + fx * area[2]) * 2.0 - 1.0, (area[1] + fy * area[3]) * 2.0 - 1.0]
        };
        // The same mapping as a matrix (columns), for the text shader. Depth is flattened to the middle of the
        // range: the interface is drawn in order without depth testing, and objects placed in front of or behind
        // the canvas plane (non-zero z) must not be clipped.
        let sx = 2.0 * area[2] / c.width;
        let sy = -2.0 * area[3] / c.height;
        let canvas_to_ndc = [
            [sx, 0.0, 0.0, 0.0],
            [0.0, sy, 0.0, 0.0],
            [0.0, 0.0, 0.0, 0.0],
            [2.0 * area[0] + area[2] - 1.0, 2.0 * area[1] + area[3] - 1.0, 0.5, 1.0],
        ];
        let mut out = Vec::new();
        let white = self.white(gfx)?;
        let push = |out: &mut Vec<DrawItem>, q: &UiQuad, tex: TextureId| {
            out.push(DrawItem::Sprite(Sprite {
                tex,
                pos: q.pos.map(to_ndc),
                uv: q.uv,
                color: q.color.to_f32(),
            }));
        };
        for d in ui.draw_list() {
            match d {
                UiDraw::Quad(q) => {
                    let tex = match &q.texture {
                        Some(r) => match self.texture(gfx, r) {
                            Some(t) => t,
                            None => continue,
                        },
                        None => white,
                    };
                    push(&mut out, &q, tex);
                }
                UiDraw::Text(t) => {
                    let view = CanvasView { canvas_to_ndc, screen: gfx.extent() };
                    let (textures, failed) = (&mut self.textures, &mut self.failed);
                    let mut atlas = |a: &mut Assets, r: &TextureRef| texture(a, textures, failed, gfx, r);
                    let mut a = self.assets.borrow_mut();
                    if self.player.is_none() {
                        // Type trees for the player files come from a font bundle (materials and shaders).
                        let game = a.game().to_path_buf();
                        let p = a.bundle("font-en").and_then(|b| PlayerFiles::open(&game, b));
                        self.player = Some(p.inspect_err(|e| log::error!("player files: {e:#}")).ok());
                    }
                    let player = self.player.as_mut().and_then(|p| p.as_mut());
                    for draw in self.text.draw(&mut a, player, ui, &t, lang, view, &mut atlas) {
                        out.push(DrawItem::Text(Box::new(draw)));
                    }
                }
            }
        }
        // The fade layer covers the whole canvas.
        let [r, g, b, a] = fade;
        let color =
            Color32::from(Color::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 128.0));
        let (hw, hh) = (c.width * 0.5, c.height * 0.5);
        let layer = UiQuad {
            texture: None,
            pos: [[-hw, -hh], [-hw, hh], [hw, -hh], [hw, hh]],
            uv: [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]],
            color,
        };
        push(&mut out, &layer, white);
        Ok(out)
    }
}

/// A texture of a bundle, uploaded on first use (failures are reported once).
fn texture(
    assets: &mut Assets,
    textures: &mut HashMap<TextureRef, TextureId>,
    failed: &mut HashSet<TextureRef>,
    gfx: &mut Gfx,
    r: &TextureRef,
) -> Option<TextureId> {
    if let Some(&t) = textures.get(r) {
        return Some(t);
    }
    if failed.contains(r) {
        return None;
    }
    let up = (|| {
        let tex = assets.bundle(&r.bundle)?.texture2d(r.obj)?;
        gfx.upload_texture(&tex)
    })();
    match up {
        Ok(t) => {
            textures.insert(r.clone(), t);
            Some(t)
        }
        Err(e) => {
            log::error!("texture {}:{}: {e:#}", r.bundle, r.obj.path_id);
            failed.insert(r.clone());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_keeps_16_9() {
        assert_eq!(canvas_area((1920, 1080), Canvas::DESIGN), [0.0, 0.0, 1.0, 1.0]);
        // 32:9: bars on both sides, half the width each.
        let a = canvas_area((3840, 1080), Canvas::DESIGN);
        assert!((a[0] - 0.25).abs() < 1e-6 && (a[2] - 0.5).abs() < 1e-6);
        // 4:3: bars at the top and bottom.
        let a = canvas_area((1440, 1080), Canvas::DESIGN);
        assert!((a[3] - 0.75).abs() < 1e-6 && (a[1] - 0.125).abs() < 1e-6);
    }
}
