//! UI animation: clips that drive image colors, rectangle positions, group alpha and object activity, and the
//! animator component that plays them.
//!
//! A clip stores curves in three groups, in this binding order: streamed (piecewise cubic segments), dense
//! (sampled frames, not supported: no UI clip uses them) and constant. Each curve is bound to a property of an object below the animator by two
//! CRC-32 hashes: the object's path relative to the animator and the property name.

use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;

/// CRC-32 (IEEE), as used for the binding hashes.
pub fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    !c
}

/// Image color channels an animation can drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorChannel {
    R,
    G,
    B,
    A,
}

impl ColorChannel {
    fn from_hash(h: u32) -> Option<Self> {
        [("m_Color.r", Self::R), ("m_Color.g", Self::G), ("m_Color.b", Self::B), ("m_Color.a", Self::A)]
            .into_iter()
            .find(|(n, _)| crc32(n.as_bytes()) == h)
            .map(|(_, c)| c)
    }
}

/// A property an animation can drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Property {
    ImageColor(ColorChannel),
    AnchoredX,
    AnchoredY,
    GroupAlpha,
    Active,
}

/// Unity class ids of the animated components.
const CLASS_GAME_OBJECT: u64 = 1;
const CLASS_MONO_BEHAVIOUR: u64 = 114;
const CLASS_RECT_TRANSFORM: u64 = 224;
const CLASS_CANVAS_GROUP: u64 = 225;

impl Property {
    /// The property bound by class id `class` and attribute hash `attr`.
    fn from_binding(class: u64, attr: u32) -> Option<Self> {
        let is = |name: &str| crc32(name.as_bytes()) == attr;
        match class {
            CLASS_MONO_BEHAVIOUR => ColorChannel::from_hash(attr).map(Self::ImageColor),
            CLASS_RECT_TRANSFORM if is("m_AnchoredPosition.x") => Some(Self::AnchoredX),
            CLASS_RECT_TRANSFORM if is("m_AnchoredPosition.y") => Some(Self::AnchoredY),
            CLASS_CANVAS_GROUP if is("m_Alpha") => Some(Self::GroupAlpha),
            CLASS_GAME_OBJECT if is("m_IsActive") => Some(Self::Active),
            _ => None,
        }
    }
}

/// One animated property: which object (path hash) and which property.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub path: u32,
    pub property: Property,
}

/// A cubic segment of a streamed curve, starting at `time`: `((a·dt + b)·dt + c)·dt + d`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Key {
    time: f32,
    coeff: [f32; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnimClip {
    pub start: f32,
    pub stop: f32,
    pub looping: bool,
    /// Per streamed curve, its segments in time order.
    streamed: Vec<Vec<Key>>,
    constants: Vec<f32>,
    pub bindings: Vec<Binding>,
}

fn f32_of(v: &Value) -> Result<f32> {
    v.as_f64().map(|f| f as f32).context("expected a number")
}

fn words(v: &Value) -> Result<Vec<u32>> {
    v.as_array()
        .context("expected an array")?
        .iter()
        .map(|w| w.as_u64().map(|x| x as u32).context("word"))
        .collect()
}

impl AnimClip {
    /// Load an `AnimationClip` object value.
    pub fn load(v: &Value) -> Result<Self> {
        let mc = &v["m_MuscleClip"];
        let clip = &mc["m_Clip"]["data"];
        let sc = &clip["m_StreamedClip"];
        let curve_count = sc["curveCount"].as_u64().context("streamed curve count")? as usize;
        let mut streamed = vec![Vec::new(); curve_count];
        let data = words(&sc["data"])?;
        let mut i = 0;
        while i < data.len() {
            ensure!(i + 2 <= data.len(), "truncated streamed frame");
            let time = f32::from_bits(data[i]);
            let n = data[i + 1] as usize;
            i += 2;
            for _ in 0..n {
                ensure!(i + 5 <= data.len(), "truncated streamed key");
                let curve = data[i] as usize;
                ensure!(curve < curve_count, "streamed key for curve {curve}");
                let coeff = [data[i + 1], data[i + 2], data[i + 3], data[i + 4]].map(f32::from_bits);
                streamed[curve].push(Key { time, coeff });
                i += 5;
            }
        }
        let dense_curves = clip["m_DenseClip"]["m_CurveCount"].as_u64().context("dense curves")? as usize;
        ensure!(dense_curves == 0, "dense (sampled) curves are not supported");
        let constants: Vec<f32> = clip["m_ConstantClip"]["data"]
            .as_array()
            .context("constants")?
            .iter()
            .map(f32_of)
            .collect::<Result<_>>()?;
        let mut bindings = Vec::new();
        for b in v["m_ClipBindingConstant"]["genericBindings"].as_array().context("bindings")? {
            let attr = b["attribute"].as_u64().context("attribute")? as u32;
            let path = b["path"].as_u64().context("path")? as u32;
            let class = b["typeID"].as_u64().context("binding class")?;
            let Some(property) = Property::from_binding(class, attr) else {
                bail!("animated property {attr:#x} of class {class} is not supported");
            };
            ensure!(b["isPPtrCurve"].as_u64() == Some(0), "object reference curves are not supported");
            bindings.push(Binding { path, property });
        }
        ensure!(
            bindings.len() == curve_count + dense_curves + constants.len(),
            "{} bindings for {} curves",
            bindings.len(),
            curve_count + dense_curves + constants.len()
        );
        Ok(Self {
            start: f32_of(&mc["m_StartTime"])?,
            stop: f32_of(&mc["m_StopTime"])?,
            looping: mc["m_LoopTime"].as_bool().unwrap_or(false),
            streamed,
            constants,
            bindings,
        })
    }

    /// Value of every binding at clip time `t` (already wrapped into the clip's range).
    pub fn sample(&self, t: f32) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.bindings.len());
        for keys in &self.streamed {
            let k = keys.iter().rev().find(|k| k.time <= t).or(keys.first());
            out.push(k.map_or(0.0, |k| {
                let dt = t - k.time;
                let [a, b, c, d] = k.coeff;
                ((a * dt + b) * dt + c) * dt + d
            }));
        }
        out.extend_from_slice(&self.constants);
        out
    }

    /// Clip time after `elapsed` seconds of play.
    pub fn time_at(&self, elapsed: f32) -> f32 {
        let len = self.stop - self.start;
        if len <= 0.0 {
            return self.start;
        }
        if self.looping { self.start + elapsed.rem_euclid(len) } else { self.start + elapsed.min(len) }
    }
}

/// An animator with a single state playing one clip (the only kind the UI uses).
#[derive(Clone, Debug, PartialEq)]
pub struct AnimatorDef {
    /// Name hash of the state.
    pub state: u32,
    pub speed: f32,
    pub clip: AnimClip,
    /// Keep the playing state when disabled (else re-enabling starts the default state over).
    pub keep_state_on_disable: bool,
    pub enabled: bool,
}

impl AnimatorDef {
    /// Build from an `Animator` component, its controller and the controller's clips (object values).
    pub fn load(animator: &Value, controller: &Value, clips: &[Value]) -> Result<Self> {
        let c = &controller["m_Controller"];
        let layers = c["m_LayerArray"].as_array().context("layers")?;
        let machines = c["m_StateMachineArray"].as_array().context("state machines")?;
        ensure!(layers.len() == 1 && machines.len() == 1, "only single-layer controllers are supported");
        let sm = &machines[0]["data"];
        let states = sm["m_StateConstantArray"].as_array().context("states")?;
        let default = sm["m_DefaultState"].as_u64().context("default state")? as usize;
        let st = &states.get(default).context("default state index")?["data"];
        let trees = st["m_BlendTreeConstantArray"].as_array().context("blend trees")?;
        ensure!(trees.len() == 1, "only single-motion states are supported");
        let nodes = trees[0]["data"]["m_NodeArray"].as_array().context("blend nodes")?;
        ensure!(nodes.len() == 1, "blend trees are not supported");
        let clip_id = nodes[0]["data"]["m_ClipID"].as_u64().context("clip id")? as usize;
        let clip = AnimClip::load(clips.get(clip_id).context("clip index")?)?;
        Ok(Self {
            state: st["m_NameID"].as_u64().context("state name")? as u32,
            speed: f32_of(&st["m_Speed"])?,
            clip,
            keep_state_on_disable: animator["m_KeepAnimatorControllerStateOnDisable"]
                .as_bool()
                .unwrap_or(false),
            enabled: animator["m_Enabled"].as_u64() != Some(0),
        })
    }
}

/// Play state of one animator instance.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimatorState {
    pub def: std::rc::Rc<AnimatorDef>,
    pub enabled: bool,
    /// Seconds played in the current state.
    pub elapsed: f32,
}

impl AnimatorState {
    pub fn new(def: std::rc::Rc<AnimatorDef>) -> Self {
        let enabled = def.enabled;
        Self { def, enabled, elapsed: 0.0 }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if enabled && !self.enabled && !self.def.keep_state_on_disable {
            // Re-enabled without keeping its state: the default state starts over.
            self.elapsed = 0.0;
        }
        self.enabled = enabled;
    }

    /// Start state `name_hash` at normalized time `t`.
    pub fn play(&mut self, name_hash: u32, t: f32) {
        if name_hash != self.def.state {
            log::warn!("animator has no state {name_hash:#x}");
            return;
        }
        self.elapsed = t * (self.def.clip.stop - self.def.clip.start);
    }

    pub fn advance(&mut self, dt: f32) {
        if self.enabled {
            self.elapsed += dt * self.def.speed;
        }
    }

    /// Current values of every binding.
    pub fn sample(&self) -> Vec<f32> {
        self.def.clip.sample(self.def.clip.time_at(self.elapsed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_of_binding_names() {
        assert_eq!(crc32(b"m_Color.a"), 304_273_561);
        assert_eq!(crc32(b"title_press"), 3_174_419_146);
    }
}
