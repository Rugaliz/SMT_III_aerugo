//! Text components of the UI tree as glyph meshes for the renderer: fonts and materials resolved the way the
//! original resolves them per language, laid out, and given the text shader's parameters.

use std::collections::HashMap;
use std::rc::Rc;

use aerugo_gfx::{TextDraw, TextGlobals, TextVertex};
use aerugo_ui::font::FontAsset;
use aerugo_ui::material::{BRIGHTNESS_LEVELS, CONTRAST_LEVELS, DEFAULT_LEVEL, TextMaterial};
use aerugo_ui::text::{GlyphQuad, MaterialMetrics, TextStyle, layout};
use aerugo_ui::{Affine, AssetRef, NodeId, TextureRef, UiText, UiTree};
use aerugo_unity::{ObjRef, ReadOpts};
use anyhow::{Context, Result, bail};

use crate::assets::Assets;
use crate::player_files::PlayerFiles;

/// The font bundles, by language group.
const FONT_BUNDLES: [&str; 4] = ["font-ja", "font-en", "font-zh", "font-ko"];

/// The font a text switching fonts with the language gets: bundle, asset name, line spacing override.
fn language_font(lang: usize) -> Option<(&'static str, &'static str, Option<f32>)> {
    match lang {
        1..=5 => Some(("font-en", "FOT-MatisseProN-B SDF", None)),
        6 => Some(("font-zh", "DFNMingXBold-B5 SDF", Some(0.0))),
        7 => Some(("font-ko", "AsiaKMJ-B SDF", Some(-10.0))),
        // Japanese keeps the prefab's font.
        _ => None,
    }
}

/// An object in a bundle.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct BundleObj {
    bundle: Rc<str>,
    obj: ObjRef,
}

struct Font {
    asset: FontAsset,
    bundle: Rc<str>,
}

struct Material {
    mat: TextMaterial,
    metrics: MaterialMetrics,
}

/// What one text node resolved to.
#[derive(Clone)]
struct Resolved {
    font: BundleObj,
    material: BundleObj,
    line_spacing: Option<f32>,
}

#[derive(Clone, PartialEq)]
struct LayoutKey {
    text: String,
    style: TextStyle,
    rect: [f32; 4],
    scale_ratio: f32,
}

/// What a node's resolution depends on: the node and its font, material and font-switching settings.
type ResolveKey = (NodeId, Option<AssetRef>, Option<AssetRef>, bool);

/// Where the canvas lands on the screen.
#[derive(Clone, Copy, Debug)]
pub struct CanvasView {
    /// Canvas units to clip space (columns).
    pub canvas_to_ndc: [[f32; 4]; 4],
    /// Window size in pixels.
    pub screen: (u32, u32),
}

#[derive(Default)]
pub struct TextRenderer {
    fonts: HashMap<BundleObj, Rc<Font>>,
    materials: HashMap<(BundleObj, bool), Rc<Material>>,
    resolved: HashMap<ResolveKey, Option<Resolved>>,
    layouts: HashMap<NodeId, (LayoutKey, Rc<Vec<GlyphQuad>>)>,
    /// Materials named by text tags (`None`: could not be loaded).
    tag_materials: HashMap<String, Option<Rc<Material>>>,
}

impl std::fmt::Debug for TextRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextRenderer")
            .field("fonts", &self.fonts.len())
            .field("layouts", &self.layouts.len())
            .finish()
    }
}

impl TextRenderer {
    /// The bundle and file index holding serialized file `file` (searched in the open bundles, then the font
    /// bundles).
    fn locate(assets: &mut Assets, file: &str) -> Option<(Rc<str>, usize)> {
        if let Some((b, node)) = assets.opened_file(file) {
            return Some((Rc::from(b), node));
        }
        for b in FONT_BUNDLES {
            let Ok(ar) = assets.bundle(b) else { continue };
            if let Some(node) = ar.bundle.nodes.iter().position(|n| n.path == file) {
                return Some((Rc::from(b), node));
            }
        }
        None
    }

    fn font(&mut self, assets: &mut Assets, at: &BundleObj) -> Result<Rc<Font>> {
        if let Some(f) = self.fonts.get(at) {
            return Ok(f.clone());
        }
        let ar = assets.bundle(&at.bundle)?;
        let asset = FontAsset::load(ar, at.obj)?;
        let f = Rc::new(Font { asset, bundle: at.bundle.clone() });
        self.fonts.insert(at.clone(), f.clone());
        Ok(f)
    }

    /// A font asset by name in a bundle.
    fn font_by_name(&mut self, assets: &mut Assets, bundle: &str, name: &str) -> Result<BundleObj> {
        let ar = assets.bundle(bundle)?;
        let want = format!("/{}.asset", name.to_lowercase());
        let refs: Vec<ObjRef> =
            ar.container.iter().filter(|(p, _)| p.ends_with(&want)).map(|(_, r)| *r).collect();
        for r in refs {
            let at = BundleObj { bundle: Rc::from(bundle), obj: r };
            if self.font(assets, &at).is_ok() {
                return Ok(at);
            }
        }
        bail!("no font asset {name} in {bundle}")
    }

    fn material(&mut self, assets: &mut Assets, at: &BundleObj, extra_padding: bool) -> Result<Rc<Material>> {
        let key = (at.clone(), extra_padding);
        if let Some(m) = self.materials.get(&key) {
            return Ok(m.clone());
        }
        let ar = assets.bundle(&at.bundle)?;
        let mut mat = TextMaterial::load(ar, at.obj)?;
        let metrics = MaterialMetrics::of(&mut mat, extra_padding);
        let m = Rc::new(Material { mat, metrics });
        self.materials.insert(key, m.clone());
        Ok(m)
    }

    /// The font and material a text node draws with: the language's font when the node switches fonts, and
    /// the font's own material unless the node's material samples that font's atlas.
    fn resolve(&mut self, assets: &mut Assets, t: &aerugo_ui::Text, lang: usize) -> Result<Resolved> {
        let mut line_spacing = None;
        let font = match (t.change_font, language_font(lang)) {
            (true, Some((bundle, name, spacing))) => {
                line_spacing = spacing;
                self.font_by_name(assets, bundle, name)?
            }
            _ => {
                let r = t.font.as_ref().context("text without a font")?;
                let (bundle, node) =
                    Self::locate(assets, &r.file).with_context(|| format!("file {}", r.file))?;
                BundleObj { bundle, obj: ObjRef { node, path_id: r.path_id } }
            }
        };
        let f = self.font(assets, &font)?;
        let atlas = f.asset.atlases.first().map(|&obj| BundleObj { bundle: f.bundle.clone(), obj });
        let own = f.asset.material.map(|obj| BundleObj { bundle: f.bundle.clone(), obj });
        let mut material = None;
        if let Some(r) = &t.material {
            match Self::locate(assets, &r.file) {
                Some((bundle, node)) => {
                    let at = BundleObj { bundle, obj: ObjRef { node, path_id: r.path_id } };
                    let m = self.material(assets, &at, t.style.extra_padding)?;
                    let tex = m.mat.main_tex.map(|obj| BundleObj { bundle: at.bundle.clone(), obj });
                    if tex.is_some() && tex == atlas {
                        material = Some(at);
                    }
                }
                None => log::warn!("text material in file {} is not supported yet", r.file),
            }
        }
        let material = material.or(own).context("font without a material")?;
        Ok(Resolved { font, material, line_spacing })
    }

    /// The glyph mesh of one text item, or `None` when it cannot be drawn (reported once).
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        assets: &mut Assets,
        player: Option<&mut PlayerFiles>,
        ui: &UiTree,
        item: &UiText,
        lang: usize,
        view: CanvasView,
        atlas_texture: &mut dyn FnMut(&mut Assets, &TextureRef) -> Option<aerugo_gfx::TextureId>,
    ) -> Vec<TextDraw> {
        self.draw_inner(assets, player, ui, item, lang, view, atlas_texture).unwrap_or_default()
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_inner(
        &mut self,
        assets: &mut Assets,
        player: Option<&mut PlayerFiles>,
        ui: &UiTree,
        item: &UiText,
        lang: usize,
        view: CanvasView,
        atlas_texture: &mut dyn FnMut(&mut Assets, &TextureRef) -> Option<aerugo_gfx::TextureId>,
    ) -> Option<Vec<TextDraw>> {
        let t = ui.text(item.node)?;
        let key = (item.node, t.font.clone(), t.material.clone(), t.change_font);
        if !self.resolved.contains_key(&key) {
            let r = self.resolve(assets, t, lang);
            if let Err(e) = &r {
                log::error!("text {:?}: {e:#}", ui.node(item.node).name);
            }
            self.resolved.insert(key.clone(), r.ok());
        }
        let r = self.resolved.get(&key)?.clone()?;
        let font = self.fonts.get(&r.font)?.clone();
        let mat =
            self.materials.get(&(r.material.clone(), t.style.extra_padding)).cloned().or_else(|| {
                self.material(assets, &r.material, t.style.extra_padding)
                    .inspect_err(|e| log::error!("text material: {e:#}"))
                    .ok()
            })?;
        let mut style = t.style.clone();
        if let Some(s) = r.line_spacing {
            style.line_spacing = s;
        }
        let lk = LayoutKey { text: t.text.clone(), style, rect: item.rect, scale_ratio: item.scale_ratio };
        let quads = match self.layouts.get(&item.node) {
            Some((k, q)) if *k == lk => q.clone(),
            _ => {
                if lk.style.rich_text {
                    self.load_tag_materials(player, &lk.text, lk.style.extra_padding);
                }
                let tags = &self.tag_materials;
                let lookup = |name: &str| tags.get(name).cloned().flatten().map(|m| m.metrics);
                let q = Rc::new(layout(
                    &lk.text,
                    &font.asset,
                    mat.metrics,
                    &lookup,
                    &lk.style,
                    lk.rect,
                    lk.scale_ratio,
                ));
                self.layouts.insert(item.node, (lk, q.clone()));
                q
            }
        };
        let atlas_obj = *font.asset.atlases.first()?;
        // Tag materials take the language's atlas (as the original sets them up), which is the font's.
        let atlas = atlas_texture(assets, &TextureRef { bundle: font.bundle.clone(), obj: atlas_obj })?;
        // The text's own mesh first, then one per tag material in order of first use.
        let mut groups: Vec<Option<String>> = vec![None];
        for q in quads.iter() {
            if !groups.contains(&q.material) {
                groups.push(q.material.clone());
            }
        }
        let mut out = Vec::new();
        for g in groups {
            let m = match &g {
                None => mat.clone(),
                Some(name) => self.tag_materials.get(name).cloned().flatten()?,
            };
            let vertices = vertices(quads.iter().filter(|q| q.material == g), item.alpha);
            if !vertices.is_empty() {
                out.push(TextDraw { atlas, globals: globals(&m, &item.to_canvas, view), vertices });
            }
        }
        Some(out)
    }

    /// Load the materials named by the text's material tags (once per name).
    fn load_tag_materials(&mut self, mut player: Option<&mut PlayerFiles>, text: &str, extra_padding: bool) {
        let mut rest = text;
        while let Some(start) = rest.find('<') {
            rest = &rest[start + 1..];
            let Some(end) = rest.find('>') else { break };
            let tag = &rest[..end];
            rest = &rest[end..];
            let Some(name) = tag
                .get(..9)
                .filter(|p| p.eq_ignore_ascii_case("material="))
                .map(|_| tag[9..].trim_matches('"'))
            else {
                continue;
            };
            if name.eq_ignore_ascii_case("default") || self.tag_materials.contains_key(name) {
                continue;
            }
            let m = match player.as_deref_mut() {
                Some(p) => load_resource_material(p, name, extra_padding)
                    .inspect_err(|e| log::error!("text material {name}: {e:#}"))
                    .ok()
                    .map(Rc::new),
                None => None,
            };
            self.tag_materials.insert(name.to_string(), m);
        }
    }
}

/// Where text tags find materials: the font assets' resource folder.
const TAG_MATERIAL_FOLDER: &str = "Fonts & Materials/";

/// A material from the player's resources, with its shader from the file it references.
fn load_resource_material(p: &mut PlayerFiles, name: &str, extra_padding: bool) -> Result<Material> {
    let (file, id) = p.resource(&format!("{TAG_MATERIAL_FOLDER}{name}")).context("no such resource")?;
    let (class, v, externals) = p.read(&file, id, ReadOpts::default())?;
    if class != "Material" {
        bail!("resource is a {class}");
    }
    let sref = &v["m_Shader"];
    let shader_file = match sref["m_FileID"].as_i64().unwrap_or(0) {
        0 => file.clone(),
        f => externals.get(f as usize - 1).context("shader file")?.clone(),
    };
    let (_, sh, _) = p.read(
        &shader_file,
        sref["m_PathID"].as_i64().unwrap_or(0),
        ReadOpts { max_bytes: 16, ..ReadOpts::default() },
    )?;
    let mut mat = TextMaterial::from_values(&v, &sh, None);
    let metrics = MaterialMetrics::of(&mut mat, extra_padding);
    Ok(Material { mat, metrics })
}

/// Two triangles per glyph: bottom-left, top-left, top-right and top-right, bottom-right, bottom-left.
fn vertices<'a>(quads: impl Iterator<Item = &'a GlyphQuad>, alpha: f32) -> Vec<TextVertex> {
    let mut out = Vec::new();
    for q in quads {
        let v = |k: usize| {
            let mut color = q.colors[k].0;
            if alpha != 1.0 {
                // Canvas groups above the text fade it.
                color[3] = (f32::from(color[3]) * alpha).round_ties_even() as u8;
            }
            TextVertex {
                pos: [q.pos[k][0], q.pos[k][1], 0.0],
                normal: [0.0, 0.0, -1.0],
                color,
                uv0: q.uv[k],
                // Face-texture coordinates are not packed: the face and outline textures are plain white.
                uv1: [0.0, q.scale],
            }
        };
        out.extend([v(0), v(1), v(2), v(2), v(3), v(0)]);
    }
    out
}

fn mat4(a: &Affine) -> [[f32; 4]; 4] {
    let m = a.m;
    [
        [m[0][0], m[1][0], m[2][0], 0.0],
        [m[0][1], m[1][1], m[2][1], 0.0],
        [m[0][2], m[1][2], m[2][2], 0.0],
        [a.t[0], a.t[1], a.t[2], 1.0],
    ]
}

/// Inverse of an affine transform (identity when singular).
fn inverse(a: &Affine) -> [[f32; 4]; 4] {
    let m = a.m;
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if det.abs() < 1e-12 {
        return mat4(&Affine::IDENTITY);
    }
    let inv = |r0: usize, c0: usize, r1: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    let im = [
        [inv(1, 1, 2, 2) / det, -inv(0, 1, 2, 2) / det, inv(0, 1, 1, 2) / det],
        [-inv(1, 0, 2, 2) / det, inv(0, 0, 2, 2) / det, -inv(0, 0, 1, 2) / det],
        [inv(1, 0, 2, 1) / det, -inv(0, 0, 2, 1) / det, inv(0, 0, 1, 1) / det],
    ];
    let t = [0, 1, 2].map(|i| -(im[i][0] * a.t[0] + im[i][1] * a.t[1] + im[i][2] * a.t[2]));
    mat4(&Affine { m: im, t })
}

fn globals(m: &Material, to_canvas: &Affine, view: CanvasView) -> TextGlobals {
    let (canvas_to_ndc, screen) = (view.canvas_to_ndc, view.screen);
    let mat = &m.mat;
    let f = |k: &str| mat.float(k);
    let (w, h) = (screen.0 as f32, screen.1 as f32);
    TextGlobals {
        object_to_world: mat4(to_canvas),
        world_to_object: inverse(to_canvas),
        matrix_vp: canvas_to_ndc,
        projection: canvas_to_ndc,
        screen_params: [w, h, 1.0 + 1.0 / w, 1.0 + 1.0 / h],
        camera_pos: [0.0; 4],
        // The UV scroll speeds of the text materials are zero.
        time: [0.0; 4],
        face_color: mat.color("_FaceColor"),
        outline_color: mat.color("_OutlineColor"),
        underlay_color: mat.color("_UnderlayColor"),
        // Untextured face and outline: identity scale and offset.
        face_tex_st: [1.0, 1.0, 0.0, 0.0],
        outline_tex_st: [1.0, 1.0, 0.0, 0.0],
        clip_rect: mat.color("_ClipRect"),
        env_matrix: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
        p0: [f("_FaceDilate"), f("_OutlineWidth"), f("_OutlineSoftness"), f("_WeightNormal")],
        p1: [f("_WeightBold"), f("_ScaleRatioA"), f("_ScaleRatioC"), f("_GradientScale")],
        p2: [f("_UnderlayOffsetX"), f("_UnderlayOffsetY"), f("_UnderlayDilate"), f("_UnderlaySoftness")],
        p3: [f("_VertexOffsetX"), f("_VertexOffsetY"), f("_TextureWidth"), f("_TextureHeight")],
        p4: [f("_ScaleX"), f("_ScaleY"), f("_PerspectiveFilter"), 0.0],
        p5: [f("_FaceUVSpeedX"), f("_FaceUVSpeedY"), f("_OutlineUVSpeedX"), f("_OutlineUVSpeedY")],
        // The graphics settings' brightness and contrast (not ported: their default level).
        p6: [
            BRIGHTNESS_LEVELS[DEFAULT_LEVEL],
            CONTRAST_LEVELS[DEFAULT_LEVEL],
            f("_MaskSoftnessX"),
            f("_MaskSoftnessY"),
        ],
    }
}
