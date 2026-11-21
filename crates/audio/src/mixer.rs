//! Mixing decoded sounds to the output.
//!
//! Like the original, every sound plays through one of four groups (music, sound effects, voices, movie
//! sound) that feed a master. Each group and the master have a gain in decibels (0 dB = unchanged; the
//! original starts all of them at 0 dB). Each playing sound has its own linear volume.

use std::sync::{Arc, Mutex, MutexGuard};

use crate::vorbis::Decoded;

/// Output groups, in the order the original's mixer lists them after the master.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    Bgm = 0,
    Se = 1,
    Voice = 2,
    Movie = 3,
}

/// Decibels to a linear factor. -80 dB and below are silence.
pub fn db_to_gain(db: f32) -> f32 {
    if db <= -80.0 { 0.0 } else { 10f32.powf(db / 20.0) }
}

/// Handle to a playing sound. Stays valid (and reports "not playing") after the sound ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VoiceId(u64);

/// How to play a sound.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Play {
    pub group: Group,
    /// Linear volume, 1.0 = unchanged.
    pub volume: f32,
    /// Loop forever between these frames (`start..end`); `None` plays once.
    pub looping: Option<(u64, u64)>,
}

#[derive(Debug)]
struct Voice {
    id: VoiceId,
    pcm: Arc<Decoded>,
    /// Position in source frames.
    pos: f64,
    volume: f32,
    group: Group,
    looping: Option<(u64, u64)>,
    paused: bool,
}

#[derive(Debug)]
pub struct Mixer {
    rate: u32,
    voices: Vec<Voice>,
    next_id: u64,
    group_gain: [f32; 4],
    master_gain: f32,
}

impl Mixer {
    /// A mixer producing interleaved stereo at `rate` frames per second.
    pub fn new(rate: u32) -> Self {
        Self { rate, voices: Vec::new(), next_id: 0, group_gain: [1.0; 4], master_gain: 1.0 }
    }

    pub fn play(&mut self, pcm: Arc<Decoded>, p: Play) -> VoiceId {
        let id = VoiceId(self.next_id);
        self.next_id += 1;
        let frames = pcm.frames() as u64;
        let looping = p.looping.map(|(s, e)| (s.min(frames), e.min(frames))).filter(|(s, e)| s < e);
        self.voices.push(Voice {
            id,
            pcm,
            pos: 0.0,
            volume: p.volume,
            group: p.group,
            looping,
            paused: false,
        });
        id
    }

    pub fn stop(&mut self, id: VoiceId) {
        self.voices.retain(|v| v.id != id);
    }

    pub fn stop_all(&mut self) {
        self.voices.clear();
    }

    pub fn is_playing(&self, id: VoiceId) -> bool {
        self.voices.iter().any(|v| v.id == id)
    }

    fn voice(&mut self, id: VoiceId) -> Option<&mut Voice> {
        self.voices.iter_mut().find(|v| v.id == id)
    }

    pub fn set_volume(&mut self, id: VoiceId, volume: f32) {
        if let Some(v) = self.voice(id) {
            v.volume = volume;
        }
    }

    pub fn set_paused(&mut self, id: VoiceId, paused: bool) {
        if let Some(v) = self.voice(id) {
            v.paused = paused;
        }
    }

    /// Playback position in seconds.
    pub fn time(&self, id: VoiceId) -> Option<f64> {
        self.voices.iter().find(|v| v.id == id).map(|v| v.pos / v.pcm.rate as f64)
    }

    pub fn set_group_db(&mut self, g: Group, db: f32) {
        self.group_gain[g as usize] = db_to_gain(db);
    }

    pub fn set_master_db(&mut self, db: f32) {
        self.master_gain = db_to_gain(db);
    }

    /// Mix into `out` (interleaved stereo, overwritten).
    pub fn mix(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        let rate = self.rate as f64;
        for v in &mut self.voices {
            if v.paused {
                continue;
            }
            let gain = v.volume * self.group_gain[v.group as usize] * self.master_gain;
            let ch = v.pcm.channels as usize;
            let frames = v.pcm.frames();
            let step = v.pcm.rate as f64 / rate;
            let pcm = &v.pcm.pcm;
            let sample = |f: usize, c: usize| pcm[f * ch + c.min(ch - 1)];
            for o in out.chunks_exact_mut(2) {
                if let Some((start, end)) = v.looping {
                    while v.pos >= end as f64 {
                        v.pos -= (end - start) as f64;
                    }
                } else if v.pos >= frames as f64 {
                    break;
                }
                // Linear interpolation between neighbouring frames.
                let i = v.pos as usize;
                let t = (v.pos - i as f64) as f32;
                let next = match v.looping {
                    Some((start, end)) if i + 1 >= end as usize => start as usize,
                    _ => (i + 1).min(frames - 1),
                };
                for (c, s) in o.iter_mut().enumerate() {
                    let a = sample(i, c);
                    let b = sample(next, c);
                    *s += (a + (b - a) * t) * gain;
                }
                v.pos += step;
            }
        }
        self.voices.retain(|v| v.looping.is_some() || v.pos < v.pcm.frames() as f64);
    }
}

/// A mixer shared between the game and the audio output thread.
#[derive(Clone, Debug)]
pub struct SharedMixer(Arc<Mutex<Mixer>>);

impl SharedMixer {
    pub fn new(rate: u32) -> Self {
        Self(Arc::new(Mutex::new(Mixer::new(rate))))
    }

    pub fn lock(&self) -> MutexGuard<'_, Mixer> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Fill an output buffer (called by the audio thread).
    pub fn fill(&self, out: &mut [f32]) {
        self.lock().mix(out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(frames: usize, rate: u32) -> Arc<Decoded> {
        Arc::new(Decoded { rate, channels: 1, pcm: (0..frames).map(|i| i as f32).collect() })
    }

    #[test]
    fn plays_once_then_ends() {
        let mut m = Mixer::new(10);
        let id = m.play(ramp(3, 10), Play { group: Group::Se, volume: 1.0, looping: None });
        let mut out = [0.0; 8];
        m.mix(&mut out);
        assert_eq!(out, [0.0, 0.0, 1.0, 1.0, 2.0, 2.0, 0.0, 0.0]);
        assert!(!m.is_playing(id));
    }

    #[test]
    fn loops_inside_the_region() {
        let mut m = Mixer::new(10);
        let id = m.play(ramp(4, 10), Play { group: Group::Bgm, volume: 1.0, looping: Some((1, 3)) });
        let mut out = [0.0; 12];
        m.mix(&mut out);
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        assert_eq!(left, [0.0, 1.0, 2.0, 1.0, 2.0, 1.0]);
        assert!(m.is_playing(id));
    }

    #[test]
    fn gains_multiply_and_resampling_interpolates() {
        let mut m = Mixer::new(20);
        m.set_group_db(Group::Se, -6.0206);
        m.play(ramp(4, 10), Play { group: Group::Se, volume: 0.5, looping: None });
        let mut out = [0.0; 6];
        m.mix(&mut out);
        let left: Vec<f32> = out.iter().step_by(2).map(|v| (v * 1000.0).round() / 1000.0).collect();
        // Half-rate source: 0, 0.5, 1 times 0.5 volume times -6 dB (0.5).
        assert_eq!(left, [0.0, 0.125, 0.25]);
        assert_eq!(db_to_gain(-80.0), 0.0);
    }
}
