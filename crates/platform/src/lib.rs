//! SDL3 platform layer: window, events, keyboard/gamepad input, audio output.
//!
//! Input is polled every displayed frame and latched once per simulation tick. Mapping these raw
//! inputs to the game's controls is not implemented yet.

use std::collections::{HashMap, HashSet};

use anyhow::{Result, anyhow};
use sdl3::audio::{AudioCallback, AudioFormat, AudioSpec, AudioStream, AudioStreamWithCallback};
use sdl3::event::{Event, WindowEvent};
use sdl3::gamepad::{Axis, Button, Gamepad};
use sdl3::keyboard::Scancode;

/// Raw input state, kept current at render rate.
#[derive(Debug, Default)]
pub struct Input {
    pub keys: HashSet<Scancode>,
    pub buttons: HashSet<Button>,
    pub axes: HashMap<Axis, i16>,
    /// Keys/buttons that went down since the last `latch()` (consumed by the next tick).
    pub pressed_keys: HashSet<Scancode>,
    pub pressed_buttons: HashSet<Button>,
}

impl Input {
    /// Hand the edges accumulated since the previous tick to the simulation.
    pub fn latch(&mut self) -> (HashSet<Scancode>, HashSet<Button>) {
        (std::mem::take(&mut self.pressed_keys), std::mem::take(&mut self.pressed_buttons))
    }
}

#[derive(Debug, Clone, Copy)]
pub enum PlatformEvent {
    Quit,
    Resized(u32, u32),
    Key(Scancode),
}

/// Output sample rate (most of the game's sounds are 44.1 kHz).
pub const AUDIO_RATE: u32 = 44100;

/// Produces interleaved stereo `f32` output; called on the audio thread.
pub type AudioSource = Box<dyn FnMut(&mut [f32]) + Send>;

/// Audio output callback: silence until a source is attached.
struct Output {
    source: Option<AudioSource>,
    buf: Vec<f32>,
}

impl AudioCallback<f32> for Output {
    fn callback(&mut self, stream: &mut AudioStream, requested: i32) {
        self.buf.resize(requested.max(0) as usize, 0.0);
        match &mut self.source {
            Some(src) => src(&mut self.buf),
            None => self.buf.fill(0.0),
        }
        let _ = stream.put_data_f32(&self.buf);
    }
}

pub struct Platform {
    _sdl: sdl3::Sdl,
    _video: sdl3::VideoSubsystem,
    pub window: sdl3::video::Window,
    events: sdl3::EventPump,
    gamepad_sys: sdl3::GamepadSubsystem,
    gamepads: Vec<Gamepad>,
    audio: Option<AudioStreamWithCallback<Output>>,
    pub input: Input,
}

impl std::fmt::Debug for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Platform")
            .field("gamepads", &self.gamepads.len())
            .field("input", &self.input)
            .finish()
    }
}

impl Platform {
    pub fn new(title: &str, width: u32, height: u32) -> Result<Self> {
        let sdl = sdl3::init().map_err(|e| anyhow!("SDL init: {e}"))?;
        let video = sdl.video().map_err(|e| anyhow!("SDL video: {e}"))?;
        let window = video
            .window(title, width, height)
            .vulkan()
            .resizable()
            .high_pixel_density()
            .build()
            .map_err(|e| anyhow!("SDL window: {e}"))?;
        let events = sdl.event_pump().map_err(|e| anyhow!("SDL events: {e}"))?;
        let gamepad_sys = sdl.gamepad().map_err(|e| anyhow!("SDL gamepad: {e}"))?;

        let audio = sdl.audio().ok().and_then(|a| {
            let spec = AudioSpec {
                freq: Some(AUDIO_RATE as i32),
                channels: Some(2),
                format: Some(AudioFormat::f32_sys()),
            };
            match a.open_playback_stream(&spec, Output { source: None, buf: Vec::new() }) {
                Ok(s) => {
                    let _ = s.resume();
                    Some(s)
                }
                Err(e) => {
                    log::warn!("audio disabled: {e}");
                    None
                }
            }
        });

        Ok(Self {
            _sdl: sdl,
            _video: video,
            window,
            events,
            gamepad_sys,
            gamepads: vec![],
            audio,
            input: Input::default(),
        })
    }

    pub fn poll(&mut self) -> Vec<PlatformEvent> {
        let mut out = Vec::new();
        for e in self.events.poll_iter() {
            match e {
                Event::Quit { .. } => out.push(PlatformEvent::Quit),
                Event::Window { win_event: WindowEvent::PixelSizeChanged(w, h), .. } => {
                    out.push(PlatformEvent::Resized(w.max(0) as u32, h.max(0) as u32))
                }
                Event::KeyDown { scancode: Some(sc), repeat, .. } => {
                    self.input.keys.insert(sc);
                    if !repeat {
                        self.input.pressed_keys.insert(sc);
                        out.push(PlatformEvent::Key(sc));
                    }
                }
                Event::KeyUp { scancode: Some(sc), .. } => {
                    self.input.keys.remove(&sc);
                }
                Event::GamepadAdded { which, .. } => {
                    if let Ok(g) = self.gamepad_sys.open(which) {
                        log::info!("gamepad connected: {}", g.name().unwrap_or_default());
                        self.gamepads.push(g);
                    }
                }
                Event::GamepadButtonDown { button, .. } => {
                    self.input.buttons.insert(button);
                    self.input.pressed_buttons.insert(button);
                }
                Event::GamepadButtonUp { button, .. } => {
                    self.input.buttons.remove(&button);
                }
                Event::GamepadAxisMotion { axis, value, .. } => {
                    self.input.axes.insert(axis, value);
                }
                _ => {}
            }
        }
        out
    }

    /// Send `source`'s output to the speakers. Returns false if audio output is not available.
    pub fn set_audio_source(&mut self, source: AudioSource) -> bool {
        let Some(stream) = &mut self.audio else { return false };
        match stream.lock() {
            Some(mut out) => {
                out.source = Some(source);
                true
            }
            None => false,
        }
    }

    pub fn toggle_fullscreen(&mut self) {
        let fs = self.window.fullscreen_state() != sdl3::video::FullscreenType::Off;
        let _ = self.window.set_fullscreen(!fs);
    }
}
