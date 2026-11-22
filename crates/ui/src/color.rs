//! Colors as the original stores them (floats) and as it hands them to the renderer (bytes).

/// A color with float channels, normally 0..1 (values outside are kept and clamped on conversion).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const WHITE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }
}

/// A color with byte channels: what image vertices carry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Color32(pub [u8; 4]);

impl From<Color> for Color32 {
    /// Each channel is clamped to 0..1, scaled by 255 and rounded half to even.
    fn from(c: Color) -> Self {
        let ch = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round_ties_even() as u8;
        Self([ch(c.r), ch(c.g), ch(c.b), ch(c.a)])
    }
}

impl Color32 {
    /// Normalized channels for the renderer.
    pub fn to_f32(self) -> [f32; 4] {
        self.0.map(|v| v as f32 / 255.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversion_clamps_and_rounds_half_to_even() {
        // Out-of-range channels clamp: the logo code passes 255.0 for "white".
        assert_eq!(Color32::from(Color::new(255.0, -1.0, 1.0, 0.5)), Color32([255, 0, 255, 128]));
        // 0.5 * 255 = 127.5 rounds to the even 128; 1.5/255 * 255 = 1.5 rounds to 2, 2.5 to 2.
        assert_eq!(Color32::from(Color::new(1.5 / 255.0, 2.5 / 255.0, 0.0, 0.0)).0[..2], [2, 2]);
    }
}
