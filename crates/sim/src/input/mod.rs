//! The game's pad: button states built from key bits (gamepads), the keyboard's bound keys, and the checks
//! the game logic makes (held, triggered, repeating).

mod tables;

use tables::{COMMON_KEYS, KEY_PAD_MAP, KEYBOARD_PRESETS};

/// Pad buttons, in the game's numbering.
pub mod button {
    pub const RL: u32 = 0;
    pub const RR: u32 = 1;
    pub const RU: u32 = 2;
    pub const RD: u32 = 3;
    pub const L: u32 = 4;
    pub const R: u32 = 5;
    pub const U: u32 = 6;
    pub const D: u32 = 7;
    pub const L1: u32 = 8;
    pub const L2: u32 = 9;
    pub const R1: u32 = 10;
    pub const R2: u32 = 11;
    pub const START: u32 = 12;
    pub const SELECT: u32 = 13;
    pub const L3: u32 = 14;
    pub const R3: u32 = 15;
}

/// Number of pad buttons.
pub const BUTTONS: usize = 16;

/// The key bit each pad button reads (gamepad path), by button.
pub const BUTTON_BITS: [u64; BUTTONS] = [
    0x800, 0x100, 0x400, 0x200, 0x4, 0x8, 0x1, 0x2, 0x1000, 0x2000, 0x4000, 0x8000, 0x200_0000, 0x100_0000,
    0x1_0000, 0x2_0000,
];

/// Seconds before a held key starts repeating, and between repeats.
pub const KEY_REPEAT_DELAY: f32 = 0.5;
pub const KEY_REPEAT_INTERVAL: f32 = 0.05;

/// Threshold of the stick and trigger direction checks (axis units).
pub const STICK_BORDER: f32 = 0.4;
/// A stick counts as pushed from this distance from its centre (byte units, centre 128).
pub const STICK_PUSH_LENGTH: f32 = 64.0;

/// Direction bits from the sticks (`stick` 0 left, 1 right; `y` up positive).
pub fn stick_bits(stick: usize, x: f32, y: f32) -> u64 {
    let (up, down, left, right) =
        if stick == 0 { (0x10_0000, 0x20_0000, 0x40_0000, 0x80_0000) } else { (0x10, 0x20, 0x40, 0x80) };
    let mut bits = 0;
    if y <= -STICK_BORDER {
        bits |= down;
    } else if y >= STICK_BORDER {
        bits |= up;
    }
    if x <= -STICK_BORDER {
        bits |= left;
    } else if x >= STICK_BORDER {
        bits |= right;
    }
    bits
}

/// A stick axis (−1..1) as the pad's byte (centre 128), as the original converts it.
pub fn stick_byte(v: f32) -> u8 {
    let mut f = v * 128.0 + 128.0;
    if f >= 256.0 {
        f = 255.0;
    } else if f <= -256.0 {
        f = -255.0;
    }
    (f as i32) as u8
}

/// Kind of check: held, pressed this tick, or repeating.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    Press = 0,
    Trigger = 1,
    Repeat = 2,
}

impl Check {
    /// Result bit of a positive keyboard check.
    fn result(self) -> u8 {
        match self {
            Check::Press => 1,
            Check::Repeat => 2,
            Check::Trigger => 0x80,
        }
    }
}

/// Button state bytes of one pad: bit 0 held; 0x80 and 2 on the first tick held; 2 again every other tick from
/// the ninth.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PadState {
    pub map: [u8; BUTTONS],
    held_ticks: [u32; BUTTONS],
    /// Stick positions: [stick][x, y], centre 128.
    pub analog: [[u8; 2]; 2],
    /// Per stick: pushed state, marked like the buttons.
    pub analog_map: [u8; 2],
    analog_ticks: [u32; 2],
}

impl PadState {
    /// One tick with the current key bits.
    #[allow(clippy::needless_range_loop)] // three arrays indexed by button
    pub fn update(&mut self, bits: u64) {
        for b in 0..BUTTONS {
            if bits & BUTTON_BITS[b] == 0 {
                self.map[b] = 0;
                self.held_ticks[b] = 0;
            } else {
                self.map[b] = 1;
                self.held_ticks[b] += 1;
                if self.held_ticks[b] == 1 {
                    self.map[b] |= 0x80 | 2;
                }
            }
            if self.held_ticks[b] > 7 && self.held_ticks[b] & 1 == 0 {
                self.map[b] |= 2;
            }
        }
    }

    /// One tick of the sticks: [stick][x, y], each −1..1 with y up positive.
    pub fn update_sticks(&mut self, sticks: [[f32; 2]; 2]) {
        for (s, [x, y]) in sticks.into_iter().enumerate() {
            self.analog[s] = [stick_byte(x), stick_byte(y)];
            let dx = self.analog[s][0] as f32 - 128.0;
            let dy = self.analog[s][1] as f32 - 128.0;
            if (dx * dx + dy * dy).sqrt() >= STICK_PUSH_LENGTH {
                self.analog_map[s] = 1;
                self.analog_ticks[s] += 1;
                if self.analog_ticks[s] == 1 {
                    self.analog_map[s] |= 0x80 | 2;
                }
            } else {
                self.analog_map[s] = 0;
                self.analog_ticks[s] = 0;
            }
            if self.analog_ticks[s] > 7 && self.analog_ticks[s] & 1 == 0 {
                self.analog_map[s] |= 2;
            }
        }
    }

    /// Port design: the gamepad answers each kind of check from its state byte (see the input notes).
    fn check(&self, id: u32, check: Check) -> u8 {
        let v = self.map.get(id as usize).copied().unwrap_or(0);
        match check {
            Check::Press => v & 1,
            Check::Trigger => v & 0x80,
            Check::Repeat => v & 2,
        }
    }
}

/// The keyboard as the game sees it, provided by the platform layer.
pub trait Keys {
    /// Whether the key at physical key number `number` is held; `None` if the layout has no such key.
    fn held(&self, number: i32) -> Option<bool>;
    /// Any key or mouse button is held.
    fn any_key(&self) -> bool;
    /// Any key or mouse button went down this tick.
    fn any_key_down(&self) -> bool;
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Slot {
    trg: u8,
    cnt: u8,
    rep: u8,
    repf: u8,
    time: f32,
}

impl Slot {
    fn field(&self, check: Check) -> u8 {
        match check {
            Check::Press => self.cnt,
            Check::Trigger => self.trg,
            Check::Repeat => self.rep,
        }
    }

    /// One tick of a bound key (`held`), with `dt` seconds since the previous one.
    fn update(&mut self, held: bool, dt: f32) {
        let old = self.cnt;
        self.cnt = held as u8;
        self.trg = (self.cnt ^ old) & self.cnt;
        if self.trg == self.cnt {
            if self.cnt == 0 {
                self.repf = 0;
                self.rep = 0;
                self.time = 0.0;
            } else {
                self.repf = self.trg;
                self.rep = self.trg;
                self.time = KEY_REPEAT_DELAY;
            }
        } else {
            if self.cnt != self.repf {
                self.repf = 0;
                self.rep = 0;
                self.time = 0.0;
            }
            self.time -= dt;
            if self.time < 0.0 {
                self.rep = self.repf;
                self.time = KEY_REPEAT_INTERVAL;
            } else {
                self.rep = 0;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct KeySet {
    slots: [Slot; 2],
    /// 1: both keys together (the first held, the second pressed).
    kind: i32,
}

/// Number of keyboard actions.
pub const ACTIONS: usize = 31;
/// Config row length.
pub const CONFIG_LEN: usize = 97;

/// The keyboard configuration and the state of every bound key.
#[derive(Clone, Debug, PartialEq)]
pub struct Keyboard {
    /// The keyboard config row: [1] preset, [4 + 2a] key 1 of action a, [0x42 + a] key 2.
    pub config: [i32; CONFIG_LEN],
    common: [KeySet; 14],
    actions: [KeySet; ACTIONS],
}

impl Default for Keyboard {
    fn default() -> Self {
        Self::new()
    }
}

impl Keyboard {
    /// Default configuration: preset 0.
    pub fn new() -> Self {
        let mut common = [KeySet::default(); 14];
        for (c, k) in common.iter_mut().zip(COMMON_KEYS) {
            c.kind = k[2];
        }
        Self { config: KEYBOARD_PRESETS[0], common, actions: [KeySet::default(); ACTIONS] }
    }

    /// Keyboard preset number of the configuration.
    pub fn preset(&self) -> i32 {
        self.config[1]
    }

    /// Key number bound to slot `slot` of common key `i` or action `a`.
    fn common_key(i: usize, slot: usize) -> i32 {
        COMMON_KEYS[i][slot]
    }

    fn action_key(&self, a: usize, slot: usize) -> i32 {
        if slot == 0 { self.config[4 + 2 * a] } else { self.config[0x42 + a] }
    }

    /// Update every bound key's state; once per tick.
    pub fn update(&mut self, keys: &dyn Keys, dt: f32) {
        fn update_set(set: &mut KeySet, key: impl Fn(usize) -> i32, keys: &dyn Keys, dt: f32) {
            for (s, slot) in set.slots.iter_mut().enumerate() {
                match keys.held(key(s)) {
                    Some(held) => slot.update(held, dt),
                    None => {
                        // An unbound or unknown key clears its slot and ends the set.
                        *slot = Slot::default();
                        break;
                    }
                }
            }
        }
        for i in 0..self.common.len() {
            update_set(&mut self.common[i], |s| Self::common_key(i, s), keys, dt);
        }
        for a in 0..ACTIONS {
            let keys_of = [self.action_key(a, 0), self.action_key(a, 1)];
            update_set(&mut self.actions[a], |s| keys_of[s], keys, dt);
        }
    }

    /// Whether action `name` (its own keys, or the common key that goes with it) passes `check`.
    fn keycheck(&self, name: usize, check: Check) -> bool {
        let Some(set) = self.actions.get(name) else { return false };
        let common = match name {
            0 => Some(0),
            1 => Some(2),
            2 => Some(3),
            3 => Some(1),
            4 => Some(4),
            5 => Some(6),
            _ => None,
        };
        let c = common.map_or(0, |i| self.common[i].slots[0].field(check));
        let c2 = if common == Some(4) { self.common[5].slots[0].field(check) } else { 0 };
        if set.kind == 1 {
            let both = set.slots[0].cnt as u32 + set.slots[1].trg as u32;
            both == 2 || c == 1 || c2 == 1
        } else {
            let any = set.slots[0].field(check) as u32 + set.slots[1].field(check) as u32;
            any != 0 || c == 1 || c2 == 1
        }
    }

    /// Extra common keys for some buttons in context 0.
    fn keycheck_other(&self, id: u32, check: Check) -> bool {
        let alt = self.preset() == 1;
        let index = match id {
            0 => Some(9),
            2 => Some(11),
            0xB => Some(if alt { 13 } else { 8 }),
            9 => Some(if alt { 12 } else { 7 }),
            0xC | 0xD => Some(10),
            _ => None,
        };
        index.is_some_and(|i| self.common[i].slots[0].field(check) == 1)
    }

    /// The keyboard's answer for pad button `id` in context `cip`. Returns the result bits and, for the
    /// right-stick actions, the analog value the keys stand for (stick 1: axis, value).
    pub fn pad_map(&self, keys: &dyn Keys, id: u32, cip: i32, check: Check) -> (u8, Option<(usize, u8)>) {
        let mut res = 0u8;
        let mut analog = None;
        let gate = if check == Check::Trigger { keys.any_key_down() } else { keys.any_key() };
        for (pos, row) in KEY_PAD_MAP.iter().enumerate() {
            let [ctx, pad, action, _] = *row;
            if pad == -1 || pad != id as i32 || ctx != cip {
                continue;
            }
            if !(is_keyboard_pos(pos) || pos >= 0x42) {
                continue;
            }
            if !gate || !self.keycheck(action as usize, check) {
                continue;
            }
            res = check.result();
            analog = match action {
                8 => Some((1, 0x00)),
                9 => Some((1, 0xFF)),
                10 => Some((0, 0x00)),
                11 => Some((0, 0xFF)),
                _ => None,
            };
            break;
        }
        // (Mouse bindings are not ported.)
        if cip == 0 && matches!(id, 0 | 2 | 8..=13) && self.keycheck_other(id, check) {
            res |= check.result();
        }
        (res, analog)
    }
}

/// Keyboard key positions of the config row (the odd positions up to 64 hold mouse bindings).
fn is_keyboard_pos(pos: usize) -> bool {
    (4..=64).contains(&pos) && pos.is_multiple_of(2)
}

/// A pad check: button `id` in context `cip` from the gamepad state (`None` while the running process may not
/// read the pad) and the keyboard. Buttons 1 and 3 swap places first, as in the original.
pub fn pad_check(
    pad: Option<&PadState>,
    kb: &Keyboard,
    keys: &dyn Keys,
    id: u32,
    cip: i32,
    check: Check,
) -> bool {
    let id = match id {
        1 => 3,
        3 => 1,
        i => i,
    };
    let gamepad = pad.map_or(0, |p| p.check(id, check));
    let (k, _) = kb.pad_map(keys, id, cip, check);
    gamepad | k != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// US layout key numbers of the default keys used below.
    const SPACE: i32 = 76;
    const RETURN: i32 = 45;
    const W: i32 = 34;

    #[derive(Default)]
    struct Kb {
        held: HashSet<i32>,
        down: bool,
    }

    impl Keys for Kb {
        fn held(&self, number: i32) -> Option<bool> {
            (number > 0).then(|| self.held.contains(&number))
        }
        fn any_key(&self) -> bool {
            !self.held.is_empty()
        }
        fn any_key_down(&self) -> bool {
            self.down
        }
    }

    #[test]
    fn default_keys_reach_their_buttons() {
        let mut kb = Keyboard::new();
        let mut keys = Kb::default();
        keys.held.insert(SPACE);
        keys.down = true;
        kb.update(&keys, 1.0 / 30.0);
        // Space is action 4, button RD in context 0; the checks swap RD with RR.
        assert!(pad_check(None, &kb, &keys, button::RR, 0, Check::Press));
        assert!(pad_check(None, &kb, &keys, button::RR, 0, Check::Trigger));
        assert!(!pad_check(None, &kb, &keys, button::RD, 0, Check::Press));
        // Held on the next tick: no longer a trigger.
        keys.down = false;
        kb.update(&keys, 1.0 / 30.0);
        assert!(!pad_check(None, &kb, &keys, button::RR, 0, Check::Trigger));
        assert!(pad_check(None, &kb, &keys, button::RR, 0, Check::Press));
        // Return goes with action 4 as a common key.
        let mut keys = Kb::default();
        keys.held.insert(RETURN);
        kb.update(&keys, 1.0 / 30.0);
        assert!(pad_check(None, &kb, &keys, button::RR, 0, Check::Press));
        // W moves up.
        let mut keys = Kb::default();
        keys.held.insert(W);
        kb.update(&keys, 1.0 / 30.0);
        assert!(pad_check(None, &kb, &keys, button::U, 0, Check::Press));
    }

    #[test]
    fn held_keys_repeat_after_the_delay() {
        let mut s = Slot::default();
        s.update(true, 0.0);
        assert_eq!((s.trg, s.rep), (1, 1));
        let mut ticks = 0;
        loop {
            s.update(true, 1.0 / 30.0);
            ticks += 1;
            if s.rep == 1 {
                break;
            }
        }
        // 0.5 s at 30 ticks per second.
        assert_eq!(ticks, 16);
    }

    #[test]
    fn sticks_become_bytes_bits_and_pushes() {
        assert_eq!((stick_byte(-1.0), stick_byte(0.0), stick_byte(1.0)), (0, 128, 255));
        assert_eq!(stick_bits(0, 0.0, 1.0), 0x10_0000);
        assert_eq!(stick_bits(1, -0.5, -0.5), 0x20 | 0x40);
        let mut p = PadState::default();
        p.update_sticks([[0.0, 0.49], [0.0, 0.0]]);
        assert_eq!(p.analog_map[0], 0);
        p.update_sticks([[0.0, 0.5], [0.0, 0.0]]);
        assert_eq!(p.analog_map[0], 0x83);
    }

    #[test]
    fn pad_buttons_mark_first_tick_and_repeats() {
        let mut p = PadState::default();
        p.update(BUTTON_BITS[button::START as usize]);
        assert_eq!(p.map[button::START as usize], 0x83);
        for _ in 0..7 {
            p.update(BUTTON_BITS[button::START as usize]);
        }
        assert_eq!(p.map[button::START as usize], 3);
    }
}
