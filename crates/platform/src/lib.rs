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

/// Audio output callback. The mixer will feed it; until then it outputs silence.
#[derive(Debug)]
struct Silence;

impl AudioCallback<f32> for Silence {
    fn callback(&mut self, stream: &mut AudioStream, requested: i32) {
        let buf = vec![0.0f32; requested.max(0) as usize];
        let _ = stream.put_data_f32(&buf);
    }
}

pub struct Platform {
    _sdl: sdl3::Sdl,
    _video: sdl3::VideoSubsystem,
    pub window: sdl3::video::Window,
    events: sdl3::EventPump,
    gamepad_sys: sdl3::GamepadSubsystem,
    gamepads: Vec<Gamepad>,
    _audio: Option<AudioStreamWithCallback<Silence>>,
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

        // The game's audio is 44.1 kHz.
        let audio = sdl.audio().ok().and_then(|a| {
            let spec =
                AudioSpec { freq: Some(44100), channels: Some(2), format: Some(AudioFormat::f32_sys()) };
            match a.open_playback_stream(&spec, Silence) {
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
            _audio: audio,
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

    pub fn toggle_fullscreen(&mut self) {
        let fs = self.window.fullscreen_state() != sdl3::video::FullscreenType::Off;
        let _ = self.window.set_fullscreen(!fs);
    }
}
