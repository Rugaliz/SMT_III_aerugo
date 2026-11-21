//! The game's movies: the ASF container, the WMV3 video and WMA v2 audio decoders, and background decoding
//! of a movie for playback.

pub mod asf;
mod bits;
pub mod stream;
pub mod vc1;
pub mod wma;

pub use asf::{Asf, AudioFormat, MediaObject, StreamKind, VideoFormat};
pub use stream::{AudioTrack, MovieStream};
pub use vc1::{Picture, Plane, Vc1Decoder};
pub use wma::WmaDecoder;
