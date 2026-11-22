//! Loading UI object trees from the game's asset bundles.

use std::collections::HashMap;
use std::rc::Rc;

use aerugo_unity::{Archive, ObjRef, ReadOpts, SerializedFile, ValueReader};
use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::anim::AnimatorDef;
use crate::color::Color;
use crate::layout::{CanvasGroup, LayoutElement, LayoutGroup, SizeFitter};
use crate::text::TextStyle;
use crate::tree::{AssetRef, Image, RectTransform, Sprite, Text};

/// A texture inside a bundle: the bundle's name and the object.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TextureRef {
    pub bundle: Rc<str>,
    pub obj: ObjRef,
}

/// One node of a loaded prefab. Indices refer to [`Prefab::nodes`].
#[derive(Clone, Debug)]
pub struct PrefabNode {
    pub name: String,
    pub active: bool,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub transform: RectTransform,
    pub image: Option<Image>,
    pub text: Option<Text>,
    pub animator: Option<Rc<AnimatorDef>>,
    pub canvas_group: Option<CanvasGroup>,
    pub layout_group: Option<LayoutGroup>,
    pub layout_element: Option<LayoutElement>,
    pub size_fitter: Option<SizeFitter>,
}

impl PrefabNode {
    pub fn new(name: &str, parent: Option<usize>, children: Vec<usize>, transform: RectTransform) -> Self {
        Self {
            name: name.to_string(),
            active: true,
            parent,
            children,
            transform,
            image: None,
            text: None,
            animator: None,
            canvas_group: None,
            layout_group: None,
            layout_element: None,
            size_fitter: None,
        }
    }
}

/// A UI object tree as stored in a bundle; node 0 is the root.
#[derive(Clone, Debug, Default)]
pub struct Prefab {
    pub nodes: Vec<PrefabNode>,
    pub sprites: Vec<Sprite>,
}

/// Objects of one serialized file, indexed by path ID.
struct File<'a> {
    sf: SerializedFile<'a>,
    index: HashMap<i64, usize>,
    node: usize,
    /// The file's name inside its bundle.
    name: Rc<str>,
}

impl<'a> File<'a> {
    fn new(ar: &'a Archive, node: usize) -> Result<Self> {
        let sf = ar.serialized(node)?;
        let index = sf.objects.iter().enumerate().map(|(i, o)| (o.path_id, i)).collect();
        let name = Rc::from(ar.bundle.nodes[node].path.as_str());
        Ok(Self { sf, index, node, name })
    }

    /// A reference to an object of this file or of one of its external files; `None` for null.
    fn asset_ref(&self, v: &Value) -> Result<Option<AssetRef>> {
        let file = v["m_FileID"].as_i64().context("reference without m_FileID")?;
        let path_id = v["m_PathID"].as_i64().context("reference without m_PathID")?;
        if path_id == 0 {
            return Ok(None);
        }
        let file = if file == 0 {
            self.name.clone()
        } else {
            let ext = self
                .sf
                .externals
                .get(file as usize - 1)
                .with_context(|| format!("no external file {file}"))?;
            Rc::from(ext.path.rsplit('/').next().unwrap_or(&ext.path))
        };
        Ok(Some(AssetRef { file, path_id }))
    }

    /// Class name and value of a local object.
    fn read(&self, path_id: i64) -> Result<(String, Value)> {
        let o =
            &self.sf.objects[*self.index.get(&path_id).with_context(|| format!("no object {path_id}"))?];
        let tree = self.sf.tree(o).with_context(|| format!("object {path_id} has no type tree"))?;
        let opts = ReadOpts { max_array: usize::MAX, max_bytes: 0, max_string: usize::MAX };
        let v = ValueReader::new(tree, self.sf.object_bytes(o)?, self.sf.big_endian, opts).read_root()?;
        Ok((self.sf.class_name(o), v))
    }
}

/// Path ID of a reference to an object in the same file; `None` for null; an error for other files.
fn local_ref(v: &Value) -> Result<Option<i64>> {
    let file = v["m_FileID"].as_i64().context("reference without m_FileID")?;
    let id = v["m_PathID"].as_i64().context("reference without m_PathID")?;
    match (file, id) {
        (_, 0) => Ok(None),
        (0, id) => Ok(Some(id)),
        (f, id) => bail!("reference to object {id} of external file {f} is not supported yet"),
    }
}

fn flag_of(v: &Value, k: &str) -> bool {
    v[k].as_u64().map(|x| x != 0).or_else(|| v[k].as_bool()).unwrap_or(false)
}

fn layout_group(v: &Value, vertical: bool) -> Result<LayoutGroup> {
    let p = &v["m_Padding"];
    let int = |x: &Value| x.as_i64().map(|i| i as i32).context("layout padding");
    Ok(LayoutGroup {
        enabled: flag_of(v, "m_Enabled"),
        vertical,
        padding: [int(&p["m_Left"])?, int(&p["m_Right"])?, int(&p["m_Top"])?, int(&p["m_Bottom"])?],
        spacing: f32_of(&v["m_Spacing"])?,
        child_alignment: v["m_ChildAlignment"].as_i64().context("child alignment")? as i32,
        control_size: [flag_of(v, "m_ChildControlWidth"), flag_of(v, "m_ChildControlHeight")],
        force_expand: [flag_of(v, "m_ChildForceExpandWidth"), flag_of(v, "m_ChildForceExpandHeight")],
        use_child_scale: [flag_of(v, "m_ChildScaleWidth"), flag_of(v, "m_ChildScaleHeight")],
        totals: [[0.0; 3]; 2],
    })
}

fn layout_element(v: &Value) -> Result<LayoutElement> {
    Ok(LayoutElement {
        enabled: flag_of(v, "m_Enabled"),
        ignore_layout: flag_of(v, "m_IgnoreLayout"),
        min: [f32_of(&v["m_MinWidth"])?, f32_of(&v["m_MinHeight"])?],
        preferred: [f32_of(&v["m_PreferredWidth"])?, f32_of(&v["m_PreferredHeight"])?],
        flexible: [f32_of(&v["m_FlexibleWidth"])?, f32_of(&v["m_FlexibleHeight"])?],
        priority: v["m_LayoutPriority"].as_i64().context("layout priority")? as i32,
    })
}

fn f32_of(v: &Value) -> Result<f32> {
    v.as_f64().map(|f| f as f32).with_context(|| format!("expected a number, got {v}"))
}

fn vec2(v: &Value) -> Result<[f32; 2]> {
    Ok([f32_of(&v["x"])?, f32_of(&v["y"])?])
}

fn vec3(v: &Value) -> Result<[f32; 3]> {
    Ok([f32_of(&v["x"])?, f32_of(&v["y"])?, f32_of(&v["z"])?])
}

fn quat(v: &Value) -> Result<[f32; 4]> {
    Ok([f32_of(&v["x"])?, f32_of(&v["y"])?, f32_of(&v["z"])?, f32_of(&v["w"])?])
}

fn rect(v: &Value) -> Result<[f32; 4]> {
    Ok([f32_of(&v["x"])?, f32_of(&v["y"])?, f32_of(&v["width"])?, f32_of(&v["height"])?])
}

fn transform(class: &str, v: &Value) -> Result<RectTransform> {
    let mut t = RectTransform {
        is_rect: class == "RectTransform",
        local_position: vec3(&v["m_LocalPosition"])?,
        local_rotation: quat(&v["m_LocalRotation"])?,
        local_scale: vec3(&v["m_LocalScale"])?,
        ..Default::default()
    };
    if t.is_rect {
        t.anchor_min = vec2(&v["m_AnchorMin"])?;
        t.anchor_max = vec2(&v["m_AnchorMax"])?;
        t.anchored_position = vec2(&v["m_AnchoredPosition"])?;
        t.size_delta = vec2(&v["m_SizeDelta"])?;
        t.pivot = vec2(&v["m_Pivot"])?;
    }
    Ok(t)
}

impl Prefab {
    /// Load the prefab stored at `asset_path` (a container path, compared without case) in `ar`, a bundle
    /// opened from a file named `bundle`.
    pub fn load(ar: &Archive, bundle: &str, asset_path: &str) -> Result<Prefab> {
        let want = asset_path.to_lowercase();
        let root = ar
            .container
            .iter()
            .find(|(p, _)| *p == want)
            .map(|(_, r)| *r)
            .with_context(|| format!("{bundle}: no asset {asset_path}"))?;
        let file = File::new(ar, root.node)?;
        let mut loader = Loader {
            file: &file,
            bundle: Rc::from(bundle),
            prefab: Prefab::default(),
            sprites: HashMap::new(),
        };
        loader.game_object(root.path_id, None)?;
        Ok(loader.prefab)
    }
}

struct Loader<'f, 'a> {
    file: &'f File<'a>,
    bundle: Rc<str>,
    prefab: Prefab,
    sprites: HashMap<i64, usize>,
}

impl Loader<'_, '_> {
    fn game_object(&mut self, id: i64, parent: Option<usize>) -> Result<usize> {
        let (class, go) = self.file.read(id)?;
        if class != "GameObject" {
            bail!("object {id} is a {class}, not a GameObject");
        }
        let name = go["m_Name"].as_str().unwrap_or("").to_string();
        let index = self.prefab.nodes.len();
        self.prefab.nodes.push(PrefabNode {
            name: name.clone(),
            active: go["m_IsActive"]
                .as_bool()
                .or_else(|| go["m_IsActive"].as_u64().map(|v| v != 0))
                .unwrap_or(true),
            parent,
            children: Vec::new(),
            transform: RectTransform::default(),
            image: None,
            text: None,
            animator: None,
            canvas_group: None,
            layout_group: None,
            layout_element: None,
            size_fitter: None,
        });
        let mut children = Vec::new();
        for c in go["m_Component"].as_array().into_iter().flatten() {
            let Some(cid) = local_ref(&c["component"])? else { continue };
            let (cclass, cv) = self.file.read(cid)?;
            match cclass.as_str() {
                "RectTransform" | "Transform" => {
                    self.prefab.nodes[index].transform = transform(&cclass, &cv)?;
                    for ch in cv["m_Children"].as_array().into_iter().flatten() {
                        if let Some(t) = local_ref(ch)? {
                            children.push(t);
                        }
                    }
                }
                "CanvasGroup" => {
                    let flag =
                        |k: &str| cv[k].as_u64().map(|x| x != 0).or_else(|| cv[k].as_bool()).unwrap_or(false);
                    self.prefab.nodes[index].canvas_group = Some(CanvasGroup {
                        enabled: flag("m_Enabled"),
                        alpha: f32_of(&cv["m_Alpha"])?,
                        ignore_parent_groups: flag("m_IgnoreParentGroups"),
                    });
                }
                "Animator" => match self.animator(&cv) {
                    Ok(def) => self.prefab.nodes[index].animator = Some(Rc::new(def)),
                    // Animations of other properties than image colors are not supported yet.
                    Err(e) => log::debug!("{}: animator of {name} skipped: {e:#}", self.bundle),
                },
                "MonoBehaviour" => match self.script_name(&cv)?.as_deref() {
                    Some("Image") => {
                        let img = self.image(&cv).with_context(|| format!("image of {name}"))?;
                        self.prefab.nodes[index].image = Some(img);
                    }
                    Some("TextMeshProUGUI") => {
                        let change_font =
                            self.prefab.nodes[index].text.as_ref().is_some_and(|t| t.change_font);
                        let mut t = self.text(&cv).with_context(|| format!("text of {name}"))?;
                        t.change_font |= change_font;
                        self.prefab.nodes[index].text = Some(t);
                    }
                    Some(kind @ ("VerticalLayoutGroup" | "HorizontalLayoutGroup")) => {
                        let g = layout_group(&cv, kind == "VerticalLayoutGroup")
                            .with_context(|| format!("layout group of {name}"))?;
                        self.prefab.nodes[index].layout_group = Some(g);
                    }
                    Some("LayoutElement") => {
                        let e = layout_element(&cv).with_context(|| format!("layout element of {name}"))?;
                        self.prefab.nodes[index].layout_element = Some(e);
                    }
                    Some("ContentSizeFitter") => {
                        self.prefab.nodes[index].size_fitter = Some(SizeFitter {
                            enabled: flag_of(&cv, "m_Enabled"),
                            fit: [
                                cv["m_HorizontalFit"].as_i64().unwrap_or(0) as i32,
                                cv["m_VerticalFit"].as_i64().unwrap_or(0) as i32,
                            ],
                        });
                    }
                    Some("ChangeFont") => match &mut self.prefab.nodes[index].text {
                        Some(t) => t.change_font = true,
                        // Seen before the text component: remembered on a placeholder.
                        None => {
                            self.prefab.nodes[index].text = Some(Text {
                                enabled: false,
                                text: String::new(),
                                style: TextStyle::default(),
                                font: None,
                                material: None,
                                change_font: true,
                            })
                        }
                    },
                    _ => {}
                },
                _ => {}
            }
        }
        for t in children {
            let (_, tv) = self.file.read(t)?;
            let go_id = local_ref(&tv["m_GameObject"])?.context("transform without game object")?;
            let child = self.game_object(go_id, Some(index))?;
            self.prefab.nodes[index].children.push(child);
        }
        Ok(index)
    }

    /// An animator component with its controller and clips (all in this file).
    fn animator(&self, v: &Value) -> Result<AnimatorDef> {
        let cid = local_ref(&v["m_Controller"])?.context("animator without controller")?;
        let (class, controller) = self.file.read(cid)?;
        if class != "AnimatorController" {
            bail!("controller {cid} is a {class}");
        }
        let mut clips = Vec::new();
        for r in controller["m_AnimationClips"].as_array().into_iter().flatten() {
            let id = local_ref(r)?.context("null animation clip")?;
            let (class, clip) = self.file.read(id)?;
            if class != "AnimationClip" {
                bail!("clip {id} is a {class}");
            }
            clips.push(clip);
        }
        AnimatorDef::load(v, &controller, &clips)
    }

    /// Class name of a component's script, if the script is in this file.
    fn script_name(&self, v: &Value) -> Result<Option<String>> {
        let s = &v["m_Script"];
        if s["m_FileID"].as_i64() != Some(0) {
            return Ok(None);
        }
        let Some(id) = local_ref(s)? else { return Ok(None) };
        let (_, sv) = self.file.read(id)?;
        Ok(sv["m_ClassName"].as_str().map(str::to_string))
    }

    fn image(&mut self, v: &Value) -> Result<Image> {
        let c = &v["m_Color"];
        let sprite = match local_ref(&v["m_Sprite"])? {
            Some(id) => Some(self.sprite(id)?),
            None => None,
        };
        let flag = |k: &str| v[k].as_u64().map(|x| x != 0).or_else(|| v[k].as_bool()).unwrap_or(false);
        Ok(Image {
            enabled: flag("m_Enabled"),
            color: Color::new(f32_of(&c["r"])?, f32_of(&c["g"])?, f32_of(&c["b"])?, f32_of(&c["a"])?),
            sprite,
            kind: v["m_Type"].as_i64().unwrap_or(0) as i32,
            preserve_aspect: flag("m_PreserveAspect"),
            use_sprite_mesh: flag("m_UseSpriteMesh"),
        })
    }

    /// A text component's string, settings and font / material references.
    fn text(&self, v: &Value) -> Result<Text> {
        let flag = |k: &str| v[k].as_u64().map(|x| x != 0).or_else(|| v[k].as_bool()).unwrap_or(false);
        let num = |k: &str| v[k].as_f64().map(|x| x as f32).with_context(|| format!("text field {k}"));
        let color = |c: &Value| -> Result<Color> {
            Ok(Color::new(f32_of(&c["r"])?, f32_of(&c["g"])?, f32_of(&c["b"])?, f32_of(&c["a"])?))
        };
        let m = &v["m_margin"];
        let g = &v["m_fontColorGradient"];
        let style = TextStyle {
            font_size: num("m_fontSize")?,
            alignment: v["m_textAlignment"].as_u64().context("text alignment")? as u32,
            word_wrapping: flag("m_enableWordWrapping"),
            overflow: v["m_overflowMode"].as_i64().unwrap_or(0) as i32,
            margin: [f32_of(&m["x"])?, f32_of(&m["y"])?, f32_of(&m["z"])?, f32_of(&m["w"])?],
            color: color(&v["m_fontColor"])?,
            override_colors: flag("m_overrideHtmlColors"),
            kerning: flag("m_enableKerning"),
            rich_text: flag("m_isRichText"),
            parse_control_characters: flag("m_parseCtrlCharacters"),
            character_spacing: num("m_characterSpacing")?,
            word_spacing: num("m_wordSpacing")?,
            line_spacing: num("m_lineSpacing")?,
            paragraph_spacing: num("m_paragraphSpacing")?,
            vertex_gradient: if flag("m_enableVertexGradient") {
                Some([
                    color(&g["topLeft"])?,
                    color(&g["topRight"])?,
                    color(&g["bottomLeft"])?,
                    color(&g["bottomRight"])?,
                ])
            } else {
                None
            },
            extra_padding: flag("m_enableExtraPadding"),
            font_style: v["m_fontStyle"].as_u64().unwrap_or(0) as u32,
            right_to_left: flag("m_isRightToLeft"),
        };
        Ok(Text {
            enabled: flag("m_Enabled"),
            text: v["m_text"].as_str().unwrap_or("").to_string(),
            style,
            font: self.file.asset_ref(&v["m_fontAsset"])?,
            material: self.file.asset_ref(&v["m_sharedMaterial"])?,
            change_font: false,
        })
    }

    fn sprite(&mut self, id: i64) -> Result<usize> {
        if let Some(&i) = self.sprites.get(&id) {
            return Ok(i);
        }
        let (class, s) = self.file.read(id)?;
        if class != "Sprite" {
            bail!("object {id} is a {class}, not a Sprite");
        }
        let rd = &s["m_RD"];
        let settings = rd["settingsRaw"].as_u64().unwrap_or(0);
        // Bit 0: packed into an atlas; bits 2..5: packing rotation.
        if settings & 1 != 0 && (settings >> 2) & 0xF != 0 {
            bail!("sprite {id}: rotated atlas packing is not supported yet");
        }
        let tex_id = local_ref(&rd["texture"])?.with_context(|| format!("sprite {id} has no texture"))?;
        let (tclass, t) = self.file.read(tex_id)?;
        if tclass != "Texture2D" {
            bail!("sprite {id}: texture is a {tclass}");
        }
        let sprite = Sprite {
            texture: TextureRef {
                bundle: self.bundle.clone(),
                obj: ObjRef { node: self.file.node, path_id: tex_id },
            },
            texture_size: [f32_of(&t["m_Width"])?, f32_of(&t["m_Height"])?],
            rect: rect(&s["m_Rect"])?,
            texture_rect: rect(&rd["textureRect"])?,
            texture_rect_offset: vec2(&rd["textureRectOffset"])?,
        };
        let i = self.prefab.sprites.len();
        self.prefab.sprites.push(sprite);
        self.sprites.insert(id, i);
        Ok(i)
    }
}
