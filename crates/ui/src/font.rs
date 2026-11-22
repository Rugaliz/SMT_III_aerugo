//! TextMeshPro font assets: face metrics, glyphs and the characters that map to them.

use std::collections::HashMap;

use aerugo_unity::{Archive, ObjRef, ReadOpts};
use anyhow::{Context, Result, bail};
use serde_json::Value;

/// Line metrics of a font face, in the face's point-size units.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FaceInfo {
    pub point_size: f32,
    pub scale: f32,
    pub line_height: f32,
    pub ascent_line: f32,
    pub cap_line: f32,
    pub mean_line: f32,
    pub baseline: f32,
    pub descent_line: f32,
    pub superscript_offset: f32,
    pub superscript_size: f32,
    pub subscript_offset: f32,
    pub subscript_size: f32,
    pub underline_offset: f32,
    pub underline_thickness: f32,
    pub strikethrough_offset: f32,
    pub strikethrough_thickness: f32,
    pub tab_width: f32,
}

/// One glyph: metrics in face units and its rectangle in the atlas (pixels, origin at the bottom left).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Glyph {
    pub width: f32,
    pub height: f32,
    pub bearing_x: f32,
    pub bearing_y: f32,
    pub advance: f32,
    pub rect: [i32; 4],
    pub scale: f32,
    pub atlas: usize,
}

/// A character's glyph and scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Character {
    pub glyph: u32,
    pub scale: f32,
}

#[derive(Clone, Debug)]
pub struct FontAsset {
    pub name: String,
    pub face: FaceInfo,
    pub glyphs: HashMap<u32, Glyph>,
    pub characters: HashMap<u32, Character>,
    /// Atlas textures (same file as the asset).
    pub atlases: Vec<ObjRef>,
    pub material: Option<ObjRef>,
    pub atlas_size: (u32, u32),
    pub padding: f32,
    pub normal_style: f32,
    pub normal_spacing: f32,
    pub bold_style: f32,
    pub bold_spacing: f32,
    pub italic_style: f32,
    pub tab_size: f32,
}

fn f(v: &Value) -> f32 {
    v.as_f64().unwrap_or(0.0) as f32
}

impl FontAsset {
    /// Load the font asset object `r` of `ar`.
    pub fn load(ar: &Archive, r: ObjRef) -> Result<Self> {
        let (class, v, _) = ar.read(r, ReadOpts::default())?;
        if class != "MonoBehaviour" || v.get("m_GlyphTable").is_none() {
            bail!("object {} is not a font asset", r.path_id);
        }
        let fi = &v["m_FaceInfo"];
        let face = FaceInfo {
            point_size: f(&fi["m_PointSize"]),
            scale: f(&fi["m_Scale"]),
            line_height: f(&fi["m_LineHeight"]),
            ascent_line: f(&fi["m_AscentLine"]),
            cap_line: f(&fi["m_CapLine"]),
            mean_line: f(&fi["m_MeanLine"]),
            baseline: f(&fi["m_Baseline"]),
            descent_line: f(&fi["m_DescentLine"]),
            superscript_offset: f(&fi["m_SuperscriptOffset"]),
            superscript_size: f(&fi["m_SuperscriptSize"]),
            subscript_offset: f(&fi["m_SubscriptOffset"]),
            subscript_size: f(&fi["m_SubscriptSize"]),
            underline_offset: f(&fi["m_UnderlineOffset"]),
            underline_thickness: f(&fi["m_UnderlineThickness"]),
            strikethrough_offset: f(&fi["m_StrikethroughOffset"]),
            strikethrough_thickness: f(&fi["m_StrikethroughThickness"]),
            tab_width: f(&fi["m_TabWidth"]),
        };
        let mut glyphs = HashMap::new();
        for g in v["m_GlyphTable"].as_array().context("glyph table")? {
            let m = &g["m_Metrics"];
            let rc = &g["m_GlyphRect"];
            let index = g["m_Index"].as_u64().context("glyph index")? as u32;
            glyphs.insert(
                index,
                Glyph {
                    width: f(&m["m_Width"]),
                    height: f(&m["m_Height"]),
                    bearing_x: f(&m["m_HorizontalBearingX"]),
                    bearing_y: f(&m["m_HorizontalBearingY"]),
                    advance: f(&m["m_HorizontalAdvance"]),
                    rect: ["m_X", "m_Y", "m_Width", "m_Height"].map(|k| rc[k].as_i64().unwrap_or(0) as i32),
                    scale: f(&g["m_Scale"]),
                    atlas: g["m_AtlasIndex"].as_u64().unwrap_or(0) as usize,
                },
            );
        }
        let mut characters = HashMap::new();
        for c in v["m_CharacterTable"].as_array().context("character table")? {
            let unicode = c["m_Unicode"].as_u64().context("unicode")? as u32;
            let glyph = c["m_GlyphIndex"].as_u64().context("glyph index")? as u32;
            characters.insert(unicode, Character { glyph, scale: f(&c["m_Scale"]) });
        }
        let local = |p: &Value| -> Option<ObjRef> {
            (p["m_FileID"].as_i64() == Some(0) && p["m_PathID"].as_i64().unwrap_or(0) != 0)
                .then(|| ObjRef { node: r.node, path_id: p["m_PathID"].as_i64().unwrap_or(0) })
        };
        let atlases = v["m_AtlasTextures"].as_array().into_iter().flatten().filter_map(local).collect();
        Ok(Self {
            name: v["m_Name"].as_str().unwrap_or("").to_string(),
            face,
            glyphs,
            characters,
            atlases,
            material: local(&v["material"]),
            atlas_size: (
                v["m_AtlasWidth"].as_u64().unwrap_or(0) as u32,
                v["m_AtlasHeight"].as_u64().unwrap_or(0) as u32,
            ),
            padding: f(&v["m_AtlasPadding"]),
            normal_style: f(&v["normalStyle"]),
            normal_spacing: f(&v["normalSpacingOffset"]),
            bold_style: f(&v["boldStyle"]),
            bold_spacing: f(&v["boldSpacing"]),
            italic_style: f(&v["italicStyle"]),
            tab_size: f(&v["tabSize"]),
        })
    }

    /// Glyph of `unicode`, with the character's scale.
    pub fn glyph_of(&self, unicode: u32) -> Option<(&Glyph, f32)> {
        let c = self.characters.get(&unicode)?;
        Some((self.glyphs.get(&c.glyph)?, c.scale))
    }
}
