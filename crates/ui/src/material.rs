//! Materials of the signed-distance-field text shader: their properties, the scale ratios the original
//! derives from them, and the glyph padding those imply.

use std::collections::{HashMap, HashSet};

use aerugo_unity::{Archive, ObjRef, ReadOpts};
use anyhow::{Context, Result, bail};
use serde_json::Value;

/// Keyword that switches the ratio computation off.
const KEYWORD_RATIOS_OFF: &str = "RATIOS_OFF";
const KEYWORD_GLOW: &str = "GLOW_ON";
const KEYWORD_UNDERLAY: &str = "UNDERLAY_ON";
/// Margin the ratios keep below the gradient scale.
const RATIO_CLAMP: f32 = 1.0;

/// Brightness multipliers of the UI materials by graphics setting level.
pub const BRIGHTNESS_LEVELS: [f32; 19] =
    [0.45, 0.5, 0.55, 0.6, 0.65, 0.7, 0.75, 0.8, 0.85, 1.0, 1.15, 1.2, 1.25, 1.3, 1.35, 1.4, 1.45, 1.5, 1.55];
/// Contrast of the UI materials by graphics setting level (0: no contrast curve).
pub const CONTRAST_LEVELS: [f32; 19] =
    [2.65, 2.85, 3.05, 3.25, 3.45, 3.65, 3.85, 4.05, 4.25, 0.0, 5.5, 6.0, 6.5, 7.0, 7.5, 8.0, 8.5, 9.0, 9.5];
/// The level every graphics preset starts at.
pub const DEFAULT_LEVEL: usize = 9;

/// A text material: shader keywords and numeric properties. Property lookups follow the shader's property
/// list (a property the shader lacks reads as absent even when the material still stores a value).
#[derive(Clone, Debug)]
pub struct TextMaterial {
    pub name: String,
    pub shader_name: String,
    pub keywords: Vec<String>,
    shader_props: HashSet<String>,
    floats: HashMap<String, f32>,
    colors: HashMap<String, [f32; 4]>,
    /// The atlas texture (same file as the material).
    pub main_tex: Option<ObjRef>,
}

fn f(v: &Value) -> f32 {
    v.as_f64().unwrap_or(0.0) as f32
}

impl TextMaterial {
    /// Load material `r` (its shader must be in the same file).
    pub fn load(ar: &Archive, r: ObjRef) -> Result<Self> {
        let (class, v, _) = ar.read(r, ReadOpts::default())?;
        if class != "Material" {
            bail!("object {} is a {class}, not a Material", r.path_id);
        }
        let shader_id = v["m_Shader"]["m_PathID"].as_i64().unwrap_or(0);
        if v["m_Shader"]["m_FileID"].as_i64() != Some(0) || shader_id == 0 {
            bail!("material {}: shader in another file", r.path_id);
        }
        let shader_ref = ObjRef { node: r.node, path_id: shader_id };
        // The compiled programs are not needed here.
        let (_, sh, _) = ar
            .read(shader_ref, ReadOpts { max_bytes: 16, ..ReadOpts::default() })
            .with_context(|| format!("shader of material {}", r.path_id))?;
        let main_tex = Self::texture_ref(&v, "_MainTex").map(|path_id| ObjRef { node: r.node, path_id });
        Ok(Self::from_values(&v, &sh, main_tex))
    }

    /// Path id of a texture property's texture when it is in the material's own file.
    pub fn texture_ref(material: &Value, name: &str) -> Option<i64> {
        material["m_SavedProperties"]["m_TexEnvs"].as_array().into_iter().flatten().find_map(|e| {
            let t = &e["second"]["m_Texture"];
            (e["first"].as_str() == Some(name)
                && t["m_FileID"].as_i64() == Some(0)
                && t["m_PathID"].as_i64().unwrap_or(0) != 0)
                .then(|| t["m_PathID"].as_i64().unwrap_or(0))
        })
    }

    /// Build from a material object value and its shader's object value.
    pub fn from_values(v: &Value, shader: &Value, main_tex: Option<ObjRef>) -> Self {
        let pf = &shader["m_ParsedForm"];
        let shader_props = pf["m_PropInfo"]["m_Props"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p["m_Name"].as_str().map(str::to_string))
            .collect();
        let sp = &v["m_SavedProperties"];
        let floats = sp["m_Floats"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| Some((e["first"].as_str()?.to_string(), f(&e["second"]))))
            .collect();
        let colors = sp["m_Colors"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| {
                let c = &e["second"];
                Some((e["first"].as_str()?.to_string(), [f(&c["r"]), f(&c["g"]), f(&c["b"]), f(&c["a"])]))
            })
            .collect();
        Self {
            name: v["m_Name"].as_str().unwrap_or("").to_string(),
            shader_name: pf["m_Name"].as_str().unwrap_or("").to_string(),
            keywords: v["m_ShaderKeywords"]
                .as_str()
                .unwrap_or("")
                .split_whitespace()
                .map(str::to_string)
                .collect(),
            shader_props,
            floats,
            colors,
            main_tex,
        }
    }

    /// A material from explicit values (tests and generated materials).
    pub fn new(name: &str, keywords: &[&str], floats: &[(&str, f32)], colors: &[(&str, [f32; 4])]) -> Self {
        let mut shader_props: HashSet<String> = floats.iter().map(|(k, _)| k.to_string()).collect();
        shader_props.extend(colors.iter().map(|(k, _)| k.to_string()));
        Self {
            name: name.to_string(),
            shader_name: String::new(),
            keywords: keywords.iter().map(|k| k.to_string()).collect(),
            shader_props,
            floats: floats.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            colors: colors.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            main_tex: None,
        }
    }

    pub fn has(&self, name: &str) -> bool {
        self.shader_props.contains(name)
    }

    pub fn has_keyword(&self, k: &str) -> bool {
        self.keywords.iter().any(|x| x == k)
    }

    /// A float property (0 when the material stores none).
    pub fn float(&self, name: &str) -> f32 {
        self.floats.get(name).copied().unwrap_or(0.0)
    }

    pub fn set_float(&mut self, name: &str, v: f32) {
        self.floats.insert(name.to_string(), v);
    }

    pub fn color(&self, name: &str) -> [f32; 4] {
        self.colors.get(name).copied().unwrap_or([0.0; 4])
    }

    /// A float that only counts when the shader declares it.
    fn prop(&self, name: &str) -> f32 {
        if self.has(name) { self.float(name) } else { 0.0 }
    }

    /// Whether the shader is a distance-field one (it has the normal-weight property).
    pub fn is_sdf(&self) -> bool {
        self.has("_WeightNormal")
    }

    /// Recompute the three scale ratios from the dilation, outline, glow and underlay properties and store
    /// them in the material.
    pub fn update_ratios(&mut self) {
        let enabled = !self.has_keyword(KEYWORD_RATIOS_OFF);
        let gs = self.float("_GradientScale");
        let dilate = self.float("_FaceDilate");
        let outline = self.float("_OutlineWidth");
        let softness = self.float("_OutlineSoftness");
        let weight = self.float("_WeightNormal").max(self.float("_WeightBold")) / 4.0 + dilate;
        let t = (weight + outline + softness).max(1.0);
        let a = if enabled { (gs - RATIO_CLAMP) / (gs * t) } else { 1.0 };
        self.set_float("_ScaleRatioA", a);
        let range = (gs - RATIO_CLAMP) * weight;
        if self.has("_GlowOffset") {
            let t = (self.float("_GlowOffset") + self.float("_GlowOuter")).max(1.0);
            let b = if enabled { (gs - RATIO_CLAMP - range).max(0.0) / (gs * t) } else { 1.0 };
            self.set_float("_ScaleRatioB", b);
        }
        if self.has("_UnderlayOffsetX") {
            let off = self.float("_UnderlayOffsetX").abs().max(self.float("_UnderlayOffsetY").abs());
            let t = (off + self.float("_UnderlayDilate") + self.float("_UnderlaySoftness")).max(1.0);
            let c = if enabled { (gs - RATIO_CLAMP - range).max(0.0) / (gs * t) } else { 1.0 };
            self.set_float("_ScaleRatioC", c);
        }
    }

    /// Padding around each glyph quad, in atlas pixels: how far the dilation, outline, glow and underlay
    /// reach beyond the glyph. Updates the ratios first, as the original does.
    pub fn padding(&mut self, extra_padding: bool) -> f32 {
        if !self.has("_GradientScale") {
            // Bitmap shaders: their own padding property, plus the optional extra.
            let extra = if extra_padding { 4 } else { 0 };
            return if self.has("_Padding") { (extra + self.float("_Padding") as i32) as f32 } else { 0.0 };
        }
        self.update_ratios();
        let a = self.prop("_ScaleRatioA");
        let dilate = self.prop("_FaceDilate") * a;
        let softness = self.prop("_OutlineSoftness") * a;
        let outline = self.prop("_OutlineWidth") * a;
        let mut uniform = outline + softness + dilate;
        if self.has("_GlowOffset") && self.has_keyword(KEYWORD_GLOW) {
            let b = self.prop("_ScaleRatioB");
            let reach = dilate + (self.float("_GlowOffset") + self.float("_GlowOuter")) * b;
            uniform = uniform.max(reach);
        }
        let mut pad = [0.0f32; 4];
        if self.has("_UnderlaySoftness") && self.has_keyword(KEYWORD_UNDERLAY) {
            let c = self.prop("_ScaleRatioC");
            let ox = self.float("_UnderlayOffsetX") * c;
            let oy = self.float("_UnderlayOffsetY") * c;
            let s = dilate + (self.float("_UnderlayDilate") + self.float("_UnderlaySoftness")) * c;
            pad = [pad[0].max(s - ox), pad[1].max(s - oy), pad[2].max(s + ox), pad[3].max(s + oy)];
        }
        let extra = if extra_padding { 4.0 } else { 0.0 };
        let gs = self.float("_GradientScale");
        let pad = pad.map(|p| ((p.max(uniform) + extra).min(1.0)) * gs);
        pad[0].max(pad[1]).max(pad[2]).max(pad[3]) + 1.25
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The English title font's default material.
    pub(crate) fn title_material() -> TextMaterial {
        TextMaterial::new(
            "title",
            &["UNDERLAY_ON"],
            &[
                ("_GradientScale", 11.0),
                ("_FaceDilate", 0.1),
                ("_OutlineWidth", 0.1),
                ("_OutlineSoftness", 0.0),
                ("_WeightNormal", 0.0),
                ("_WeightBold", 0.75),
                ("_GlowOffset", 0.046),
                ("_GlowOuter", 0.05),
                ("_UnderlayOffsetX", 0.0),
                ("_UnderlayOffsetY", 0.0),
                ("_UnderlayDilate", 0.4),
                ("_UnderlaySoftness", 0.0),
                ("_ScaleRatioA", 0.0),
                ("_ScaleRatioB", 0.0),
                ("_ScaleRatioC", 0.0),
            ],
            &[],
        )
    }

    #[test]
    fn ratios_match_the_stored_values() {
        let mut m = title_material();
        m.update_ratios();
        assert!((m.float("_ScaleRatioA") - 10.0 / 11.0).abs() < 1e-6);
        // Range (0.75/4 + 0.1) * 10 = 2.875: (10 - 2.875) / 11.
        assert!((m.float("_ScaleRatioB") - 7.125 / 11.0).abs() < 1e-6);
        assert!((m.float("_ScaleRatioC") - 7.125 / 11.0).abs() < 1e-6);
    }

    #[test]
    fn underlay_sets_the_padding() {
        let mut m = title_material();
        // Dilate 0.1·A + underlay 0.4·C, times the gradient scale, plus 1.25.
        let s = 0.1 * 10.0 / 11.0 + 0.4 * 7.125 / 11.0;
        assert!((m.padding(false) - (s * 11.0 + 1.25)).abs() < 1e-4);
    }
}
