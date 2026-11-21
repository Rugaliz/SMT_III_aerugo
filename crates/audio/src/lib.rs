//! The game's sound: FSB5 sound banks of Vorbis audio, and the mixer that plays them.
//!
//! The banks leave out the Vorbis setup header, which the original's audio engine keeps in its own code.
//! [`SetupHeaders`] reads those headers from the player library of the user's installation and checks each
//! one against the CRC-32 the bank stores, so no engine data ships with the port.

mod crc32;
mod fsb5;
mod mixer;
mod pe;
mod setup;
mod vorbis;

pub use crc32::crc32;
pub use fsb5::{Bank, Loop, Sample};
pub use mixer::{Group, Mixer, Play, SharedMixer, VoiceId, db_to_gain};
pub use setup::SetupHeaders;
pub use vorbis::{Decoded, decode};
