//! ASF, the container of the game's movies: the header (stream formats) and the data packets, reassembled into
//! media objects (one video frame or one block of audio each).
//!
//! Only the features the game's files use are accepted; anything else is reported as an error.

use std::collections::HashMap;

use anyhow::{Context, Result, bail, ensure};

/// A GUID as stored in the file (Windows little-endian layout).
type Guid = [u8; 16];

const fn guid(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> Guid {
    let a = d1.to_le_bytes();
    let b = d2.to_le_bytes();
    let c = d3.to_le_bytes();
    [a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d4[0], d4[1], d4[2], d4[3], d4[4], d4[5], d4[6], d4[7]]
}

const HEADER: Guid = guid(0x75B2_2630, 0x668E, 0x11CF, [0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62, 0xCE, 0x6C]);
const DATA: Guid = guid(0x75B2_2636, 0x668E, 0x11CF, [0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62, 0xCE, 0x6C]);
const FILE_PROPERTIES: Guid =
    guid(0x8CAB_DCA1, 0xA947, 0x11CF, [0x8E, 0xE4, 0x00, 0xC0, 0x0C, 0x20, 0x53, 0x65]);
const STREAM_PROPERTIES: Guid =
    guid(0xB7DC_0791, 0xA9B7, 0x11CF, [0x8E, 0xE6, 0x00, 0xC0, 0x0C, 0x20, 0x53, 0x65]);
const AUDIO_MEDIA: Guid = guid(0xF869_9E40, 0x5B4D, 0x11CF, [0xA8, 0xFD, 0x00, 0x80, 0x5F, 0x5C, 0x44, 0x2B]);
const VIDEO_MEDIA: Guid = guid(0xBC19_EFC0, 0x5B4D, 0x11CF, [0xA8, 0xFD, 0x00, 0x80, 0x5F, 0x5C, 0x44, 0x2B]);

/// Little-endian reader over a byte slice.
#[derive(Debug, Clone)]
struct Reader<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn new(d: &'a [u8]) -> Self {
        Self { d, p: 0 }
    }
    fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let b = self.d.get(self.p..self.p + n).context("truncated ASF data")?;
        self.p += n;
        Ok(b)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.bytes(2)?.try_into()?))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into()?))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.bytes(8)?.try_into()?))
    }
    fn guid(&mut self) -> Result<Guid> {
        Ok(self.bytes(16)?.try_into()?)
    }
    /// A field whose width is given by a 2-bit code: 0 absent, 1 byte, 2 word, 3 double word.
    fn sized(&mut self, code: u8, absent: u32) -> Result<u32> {
        Ok(match code & 3 {
            0 => absent,
            1 => self.u8()?.into(),
            2 => self.u16()?.into(),
            _ => self.u32()?,
        })
    }
}

/// An audio stream's format (a wave format header).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioFormat {
    pub format_tag: u16,
    pub channels: u16,
    pub rate: u32,
    pub bytes_per_second: u32,
    pub block_align: u16,
    pub bits: u16,
    /// Codec-specific data following the header.
    pub extra: Vec<u8>,
}

/// A video stream's format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoFormat {
    pub width: u32,
    pub height: u32,
    pub fourcc: [u8; 4],
    /// Codec-specific data following the bitmap header.
    pub extra: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamKind {
    Audio,
    Video,
}

/// One complete media object of a stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaObject {
    pub stream: u8,
    pub key: bool,
    /// Presentation time in milliseconds, preroll removed.
    pub time_ms: i64,
    pub data: Vec<u8>,
}

/// A media object still being assembled from fragments.
#[derive(Debug)]
struct Partial {
    size: u32,
    data: Vec<u8>,
    key: bool,
    time_ms: i64,
}

/// A parsed ASF file.
#[derive(Debug)]
pub struct Asf<'a> {
    pub play_duration_100ns: u64,
    pub preroll_ms: u64,
    pub packet_size: u32,
    pub audio: Vec<(u8, AudioFormat)>,
    pub video: Vec<(u8, VideoFormat)>,
    data: &'a [u8],
    packets: u64,
}

impl<'a> Asf<'a> {
    pub fn parse(d: &'a [u8]) -> Result<Self> {
        let mut r = Reader::new(d);
        ensure!(r.guid()? == HEADER, "not an ASF file");
        let header_size = usize::try_from(r.u64()?)?;
        r.bytes(6)?;
        let mut a = Asf {
            play_duration_100ns: 0,
            preroll_ms: 0,
            packet_size: 0,
            audio: Vec::new(),
            video: Vec::new(),
            data: &[],
            packets: 0,
        };
        while r.p < header_size {
            let start = r.p;
            let id = r.guid()?;
            let size = usize::try_from(r.u64()?)?;
            ensure!(size >= 24 && start + size <= header_size, "bad ASF header object size");
            let mut o = Reader::new(&d[start + 24..start + size]);
            if id == FILE_PROPERTIES {
                o.guid()?;
                o.u64()?; // file size
                o.u64()?; // creation time
                o.u64()?; // packet count
                a.play_duration_100ns = o.u64()?;
                o.u64()?; // send duration
                a.preroll_ms = o.u64()?;
                let flags = o.u32()?;
                ensure!(flags & 1 == 0, "broadcast ASF files are not supported");
                let (min, max) = (o.u32()?, o.u32()?);
                ensure!(min == max && min > 0, "variable ASF packet sizes are not supported");
                a.packet_size = min;
            } else if id == STREAM_PROPERTIES {
                a.stream_properties(&mut o)?;
            }
            r.p = start + size;
        }
        ensure!(a.packet_size > 0, "ASF file without file properties");
        let mut r = Reader::new(&d[header_size..]);
        ensure!(r.guid()? == DATA, "ASF data object missing");
        let size = usize::try_from(r.u64()?)?;
        r.guid()?;
        a.packets = r.u64()?;
        r.u16()?;
        let data_end = header_size + size;
        a.data = d.get(header_size + 50..data_end).context("ASF data object out of range")?;
        ensure!(
            a.data.len() as u64 == a.packets * u64::from(a.packet_size),
            "ASF data size does not match its packets"
        );
        Ok(a)
    }

    fn stream_properties(&mut self, o: &mut Reader<'_>) -> Result<()> {
        let kind = o.guid()?;
        o.guid()?; // error correction type
        o.u64()?; // time offset
        let ts_len = o.u32()? as usize;
        let ec_len = o.u32()? as usize;
        let number = (o.u16()? & 0x7F) as u8;
        o.u32()?;
        let mut ts = Reader::new(o.bytes(ts_len)?);
        let ec = o.bytes(ec_len)?;
        if kind == AUDIO_MEDIA {
            let mut f = AudioFormat {
                format_tag: ts.u16()?,
                channels: ts.u16()?,
                rate: ts.u32()?,
                bytes_per_second: ts.u32()?,
                block_align: ts.u16()?,
                bits: ts.u16()?,
                extra: Vec::new(),
            };
            let n = ts.u16()? as usize;
            f.extra = ts.bytes(n)?.to_vec();
            if let Some(&span) = ec.first() {
                ensure!(span <= 1, "scrambled ASF audio is not supported");
            }
            self.audio.push((number, f));
        } else if kind == VIDEO_MEDIA {
            let width = ts.u32()?;
            let height = ts.u32()?;
            ts.u8()?;
            ts.u16()?;
            let bih = ts.u32()? as usize;
            ensure!(bih >= 40, "bad bitmap header size");
            ts.bytes(8)?; // width, height again
            ts.u16()?; // planes
            ts.u16()?; // bits
            let fourcc: [u8; 4] = ts.bytes(4)?.try_into()?;
            ts.bytes(20)?;
            let extra = ts.bytes(bih - 40)?.to_vec();
            self.video.push((number, VideoFormat { width, height, fourcc, extra }));
        } else {
            bail!("unsupported ASF stream type");
        }
        Ok(())
    }

    pub fn kind(&self, stream: u8) -> Option<StreamKind> {
        if self.audio.iter().any(|(n, _)| *n == stream) {
            Some(StreamKind::Audio)
        } else if self.video.iter().any(|(n, _)| *n == stream) {
            Some(StreamKind::Video)
        } else {
            None
        }
    }

    /// Every media object in file order (objects complete when their last fragment arrives).
    pub fn objects(&self) -> Result<Vec<MediaObject>> {
        let mut out = Vec::new();
        let mut pending: HashMap<u8, Partial> = HashMap::new();
        for (i, packet) in self.data.chunks_exact(self.packet_size as usize).enumerate() {
            self.packet(packet, &mut pending, &mut out).with_context(|| format!("ASF packet {i}"))?;
        }
        ensure!(pending.is_empty(), "{} media objects incomplete at the end", pending.len());
        Ok(out)
    }

    fn packet(&self, d: &[u8], pending: &mut HashMap<u8, Partial>, out: &mut Vec<MediaObject>) -> Result<()> {
        let mut r = Reader::new(d);
        let mut b = r.u8()?;
        if b & 0x80 != 0 {
            ensure!(b & 0x60 == 0, "opaque ASF error correction is not supported");
            r.bytes((b & 0x0F) as usize)?;
            b = r.u8()?;
        }
        let flags = b;
        let props = r.u8()?;
        let length = r.sized(flags >> 5, self.packet_size)? as usize;
        r.sized(flags >> 1, 0)?; // sequence
        let padding = r.sized(flags >> 3, 0)? as usize;
        ensure!(length == d.len(), "ASF packet length {length} differs from the fixed size");
        r.u32()?; // send time
        r.u16()?; // duration
        let end = d.len().checked_sub(padding).context("ASF padding larger than the packet")?;
        let (count, len_code) = if flags & 1 != 0 {
            let s = r.u8()?;
            ((s & 0x3F) as usize, s >> 6)
        } else {
            (1, 0)
        };
        for _ in 0..count {
            let num = r.u8()?;
            let stream = num & 0x7F;
            let key = num & 0x80 != 0;
            r.sized(props >> 4, 0)?; // media object number
            let offset = r.sized(props >> 2, 0)?;
            let replicated = r.sized(props, 0)? as usize;
            let rep = r.bytes(replicated)?;
            let payload_len = if flags & 1 != 0 { r.sized(len_code, 0)? as usize } else { end - r.p };
            let payload = r.bytes(payload_len)?;
            ensure!(self.kind(stream).is_some(), "payload for unknown stream {stream}");
            match replicated {
                n if n >= 8 => {
                    let size = u32::from_le_bytes(rep[0..4].try_into()?);
                    let time = u32::from_le_bytes(rep[4..8].try_into()?) as i64 - self.preroll_ms as i64;
                    let p = pending.entry(stream).or_insert_with(|| Partial {
                        size,
                        data: Vec::new(),
                        key,
                        time_ms: time,
                    });
                    ensure!(
                        offset as usize == p.data.len() && p.size == size,
                        "stream {stream}: fragment out of order"
                    );
                    p.data.extend_from_slice(payload);
                    ensure!(p.data.len() <= size as usize, "stream {stream}: object larger than its size");
                    if p.data.len() == size as usize {
                        let p = pending.remove(&stream).unwrap();
                        out.push(MediaObject { stream, key: p.key, time_ms: p.time_ms, data: p.data });
                    }
                }
                1 => {
                    let delta = rep[0] as i64;
                    let mut t = offset as i64 - self.preroll_ms as i64;
                    let mut s = Reader::new(payload);
                    while s.p < payload.len() {
                        let n = s.u8()? as usize;
                        out.push(MediaObject { stream, key, time_ms: t, data: s.bytes(n)?.to_vec() });
                        t += delta;
                    }
                }
                n => bail!("replicated data of {n} bytes is not supported"),
            }
        }
        Ok(())
    }
}
