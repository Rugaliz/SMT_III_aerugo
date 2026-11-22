//! From the window's keyboard and gamepad to the game's input: key layout tables, the keys held each tick,
//! and the gamepad's key bits.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use aerugo_sim::input::Keys;
use aerugo_unity::SerializedFile;
use anyhow::{Context, Result, bail};
use sdl3::gamepad::{Axis, Button};
use sdl3::keyboard::{Keycode, Scancode};

/// Unity class id of text assets.
const TEXT_ASSET_CLASS: i32 = 49;
/// The layout tables' names start with this.
const LAYOUT_PREFIX: &str = "KB";
/// The table whose positions name the physical keys (USB scancodes follow the US layout).
const US_LAYOUT: &str = "KB0409US";

/// One keyboard layout table: physical key number → the game's key code (a Unity KeyCode).
#[derive(Debug, Clone)]
pub struct KeyLayout {
    pub name: String,
    pub codes: HashMap<i32, u32>,
}

fn read_text_asset(b: &[u8]) -> Option<(String, &[u8])> {
    let n = u32::from_le_bytes(b.get(0..4)?.try_into().ok()?) as usize;
    let name = std::str::from_utf8(b.get(4..4 + n)?).ok()?.to_string();
    let at = (4 + n).div_ceil(4) * 4;
    let len = u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?) as usize;
    Some((name, b.get(at + 4..at + 4 + len)?))
}

/// Read every keyboard layout table from the installation (UTF-16 text: "number,label,code,display" lines).
pub fn load_layouts(game: &Path) -> Result<Vec<KeyLayout>> {
    let path = game.join("smt3hd_Data/resources.assets");
    let data = std::fs::read(&path).with_context(|| path.display().to_string())?;
    let sf = SerializedFile::parse(&data)?;
    let mut out = Vec::new();
    for o in &sf.objects {
        if sf.types[o.type_index].class_id != TEXT_ASSET_CLASS {
            continue;
        }
        let Some((name, text)) = read_text_asset(sf.object_bytes(o)?) else { continue };
        if !name.starts_with(LAYOUT_PREFIX) {
            continue;
        }
        let units: Vec<u16> = text.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let text = String::from_utf16_lossy(&units);
        let mut codes = HashMap::new();
        for line in text.lines() {
            let f: Vec<&str> = line.split(',').map(str::trim).collect();
            if let (Some(no), Some(code)) =
                (f.first().and_then(|s| s.parse().ok()), f.get(2).and_then(|s| s.parse().ok()))
            {
                codes.insert(no, code);
            }
        }
        out.push(KeyLayout { name, codes });
    }
    if out.is_empty() {
        bail!("no keyboard layout tables in {}", path.display());
    }
    Ok(out)
}

/// The game's key code as an SDL keycode. Character keys share their values; the others by name.
fn sdl_keycode(code: u32) -> Option<Keycode> {
    let named = match code {
        19 => Keycode::Pause,
        256..=265 => [
            Keycode::Kp0,
            Keycode::Kp1,
            Keycode::Kp2,
            Keycode::Kp3,
            Keycode::Kp4,
            Keycode::Kp5,
            Keycode::Kp6,
            Keycode::Kp7,
            Keycode::Kp8,
            Keycode::Kp9,
        ][(code - 256) as usize],
        266 => Keycode::KpPeriod,
        267 => Keycode::KpDivide,
        268 => Keycode::KpMultiply,
        269 => Keycode::KpMinus,
        270 => Keycode::KpPlus,
        271 => Keycode::KpEnter,
        272 => Keycode::KpEquals,
        273 => Keycode::Up,
        274 => Keycode::Down,
        275 => Keycode::Right,
        276 => Keycode::Left,
        277 => Keycode::Insert,
        278 => Keycode::Home,
        279 => Keycode::End,
        280 => Keycode::PageUp,
        281 => Keycode::PageDown,
        282..=293 => [
            Keycode::F1,
            Keycode::F2,
            Keycode::F3,
            Keycode::F4,
            Keycode::F5,
            Keycode::F6,
            Keycode::F7,
            Keycode::F8,
            Keycode::F9,
            Keycode::F10,
            Keycode::F11,
            Keycode::F12,
        ][(code - 282) as usize],
        300 => Keycode::NumLockClear,
        301 => Keycode::CapsLock,
        302 => Keycode::ScrollLock,
        303 => Keycode::RShift,
        304 => Keycode::LShift,
        305 => Keycode::RCtrl,
        306 => Keycode::LCtrl,
        307 => Keycode::RAlt,
        308 => Keycode::LAlt,
        309 => Keycode::RGui,
        310 => Keycode::LGui,
        8..=127 => return Keycode::from_u32(code),
        _ => return None,
    };
    Some(named)
}

/// Pick the table matching the system keyboard layout: the one whose letters the layout produces at the most
/// letter positions (positions named by the US table, as USB scancodes are).
pub fn choose_layout(layouts: &[KeyLayout]) -> Option<&KeyLayout> {
    let us = layouts.iter().find(|l| l.name == US_LAYOUT)?;
    let score = |l: &KeyLayout| {
        let mut n = 0;
        for (no, &us_code) in &us.codes {
            let (Some(&code), true) = (l.codes.get(no), (97..=122).contains(&us_code)) else { continue };
            let Some(sc) = Scancode::from_name(&((us_code as u8 as char).to_ascii_uppercase().to_string()))
            else {
                continue;
            };
            if Keycode::from_scancode(sc, sdl3::sys::keycode::SDL_Keymod(0), false).map(|k| k as u32)
                == Some(code)
            {
                n += 1;
            }
        }
        n
    };
    // Ties keep the US table.
    let mut best = us;
    let mut best_score = score(us);
    for l in layouts {
        let s = score(l);
        if s > best_score {
            best = l;
            best_score = s;
        }
    }
    Some(best)
}

/// The keyboard as seen for one tick.
#[derive(Debug, Default)]
pub struct KeyFrame {
    /// Keycodes (in the current layout) of the keys held.
    held: HashSet<u32>,
    any_down: bool,
    /// Key number → keycode, from the chosen layout table.
    map: HashMap<i32, u32>,
}

impl KeyFrame {
    pub fn new(layout: Option<&KeyLayout>) -> Self {
        let map = layout
            .map(|l| l.codes.iter().filter_map(|(&no, &c)| sdl_keycode(c).map(|k| (no, k as u32))).collect())
            .unwrap_or_default();
        Self { held: HashSet::new(), any_down: false, map }
    }

    /// Take the keys held now and whether any went down since the previous tick.
    pub fn set(&mut self, held: &HashSet<Scancode>, pressed: &HashSet<Scancode>) {
        let to_key = |sc: &Scancode| {
            Keycode::from_scancode(*sc, sdl3::sys::keycode::SDL_Keymod(0), false).map(|k| k as u32)
        };
        self.held = held.iter().filter_map(to_key).collect();
        self.any_down = !pressed.is_empty();
    }
}

impl Keys for KeyFrame {
    fn held(&self, number: i32) -> Option<bool> {
        self.map.get(&number).map(|k| self.held.contains(k))
    }
    fn any_key(&self) -> bool {
        !self.held.is_empty()
    }
    fn any_key_down(&self) -> bool {
        self.any_down
    }
}

use aerugo_sim::input::{STICK_BORDER, stick_bits};

/// Dead zone of the game's stick axes (its input settings).
const STICK_DEAD_ZONE: f32 = 0.19;

/// The sticks as the game reads them: [stick][x, y], −1..1, y up positive (the vertical axes are inverted in the
/// game's input settings; the raw axis grows downwards, as SDL's does). Values inside the dead zone read 0.
pub fn sticks(axes: &HashMap<Axis, i16>) -> [[f32; 2]; 2] {
    let raw = |a: Axis| {
        let v = (axes.get(&a).copied().unwrap_or(0) as f32 / i16::MAX as f32).clamp(-1.0, 1.0);
        if v.abs() < STICK_DEAD_ZONE { 0.0 } else { v }
    };
    [[raw(Axis::LeftX), -raw(Axis::LeftY)], [raw(Axis::RightX), -raw(Axis::RightY)]]
}

/// The gamepad's key bits: buttons through the original's default pad assignments, in the XInput numbering
/// (port design). The triggers form one combined axis, the left trigger on its negative side (user decision):
/// at or below -0.4 → L2, at or above 0.4 → R2.
pub fn gamepad_bits(buttons: &HashSet<Button>, axes: &HashMap<Axis, i16>) -> u64 {
    let trigger = |a: Axis| axes.get(&a).copied().unwrap_or(0).max(0) as f32 / i16::MAX as f32;
    let combined = trigger(Axis::TriggerRight) - trigger(Axis::TriggerLeft);
    let mut bits = 0;
    if combined <= -STICK_BORDER {
        bits |= 0x2000;
    }
    if combined >= STICK_BORDER {
        bits |= 0x8000;
    }
    let [l, r] = sticks(axes);
    bits | buttons_bits(buttons) | stick_bits(0, l[0], l[1]) | stick_bits(1, r[0], r[1])
}

fn buttons_bits(buttons: &HashSet<Button>) -> u64 {
    const MAP: [(Button, u64); 14] = [
        (Button::South, 0x200),
        (Button::East, 0x100),
        (Button::West, 0x800),
        (Button::North, 0x400),
        (Button::LeftShoulder, 0x1000),
        (Button::RightShoulder, 0x4000),
        (Button::Start, 0x200_0000),
        (Button::Back, 0x100_0000),
        (Button::LeftStick, 0x1_0000),
        (Button::RightStick, 0x2_0000),
        (Button::DPadUp, 0x1),
        (Button::DPadDown, 0x2),
        (Button::DPadLeft, 0x4),
        (Button::DPadRight, 0x8),
    ];
    MAP.iter().filter(|(b, _)| buttons.contains(b)).fold(0, |acc, (_, bit)| acc | bit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triggers_share_one_axis_left_negative() {
        let none = HashSet::new();
        let mut axes = HashMap::new();
        axes.insert(Axis::TriggerLeft, i16::MAX);
        assert_eq!(gamepad_bits(&none, &axes), 0x2000);
        axes.insert(Axis::TriggerRight, i16::MAX);
        // Both fully pressed cancel out on the combined axis.
        assert_eq!(gamepad_bits(&none, &axes), 0);
        axes.insert(Axis::TriggerLeft, 0);
        assert_eq!(gamepad_bits(&none, &axes), 0x8000);
        axes.insert(Axis::TriggerRight, i16::MAX / 4);
        assert_eq!(gamepad_bits(&none, &axes), 0);
    }
}
