//! Decoding a bank sample.

use anyhow::{Context, Result, anyhow};
use lewton::audio::{PreviousWindowRight, read_audio_packet_generic};
use lewton::header::{read_header_ident, read_header_setup};
use lewton::samples::InterleavedSamples;

use crate::fsb5::{Bank, Sample};
use crate::setup::SetupHeaders;

/// Vorbis block sizes as powers of two: 256 and 2048 samples. The banks don't store them; the original's
/// decoder always uses these.
const BLOCKSIZES: (u8, u8) = (8, 11);

/// PCM of one sample.
#[derive(Clone, Debug, PartialEq)]
pub struct Decoded {
    pub rate: u32,
    pub channels: u8,
    /// Interleaved, -1..1.
    pub pcm: Vec<f32>,
}

impl Decoded {
    pub fn frames(&self) -> usize {
        self.pcm.len() / self.channels as usize
    }
}

/// The identification header the bank leaves out, rebuilt from the sample's format.
fn ident_packet(channels: u8, rate: u32) -> Vec<u8> {
    let mut p = Vec::with_capacity(30);
    p.extend_from_slice(b"\x01vorbis");
    p.extend_from_slice(&0u32.to_le_bytes());
    p.push(channels);
    p.extend_from_slice(&rate.to_le_bytes());
    p.extend_from_slice(&[0; 12]);
    p.push(BLOCKSIZES.0 | (BLOCKSIZES.1 << 4));
    p.push(1);
    p
}

/// Decode a sample to its exact length in frames.
pub fn decode(bank: &Bank<'_>, sample: &Sample, headers: &SetupHeaders) -> Result<Decoded> {
    let setup = headers
        .get(sample.setup_crc)
        .with_context(|| format!("no setup header {:#010x}", sample.setup_crc))?;
    let ident = read_header_ident(&ident_packet(sample.channels, sample.rate))
        .map_err(|e| anyhow!("ident: {e:?}"))?;
    let setup = read_header_setup(setup, sample.channels, BLOCKSIZES).map_err(|e| anyhow!("setup: {e:?}"))?;
    let mut pwr = PreviousWindowRight::new();
    let frames = usize::try_from(sample.frames)?;
    let mut pcm = Vec::with_capacity(frames * sample.channels as usize);
    for (i, p) in bank.packets(sample)?.into_iter().enumerate() {
        let out: InterleavedSamples<f32> = read_audio_packet_generic(&ident, &setup, p, &mut pwr)
            .map_err(|e| anyhow!("packet {i}: {e:?}"))?;
        pcm.extend_from_slice(&out.samples);
    }
    let want = frames * sample.channels as usize;
    if pcm.len() < want {
        anyhow::bail!("decoded {} frames, the bank says {frames}", pcm.len() / sample.channels as usize);
    }
    pcm.truncate(want);
    Ok(Decoded { rate: sample.rate, channels: sample.channels, pcm })
}

#[cfg(test)]
mod tests {
    #[test]
    fn ident_packet_layout() {
        let p = super::ident_packet(2, 44100);
        assert_eq!(p.len(), 30);
        assert_eq!(p[28], 0xB8);
        assert!(lewton::header::read_header_ident(&p).is_ok());
    }
}
