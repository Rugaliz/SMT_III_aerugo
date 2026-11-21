//! Background decoding of one movie: the whole audio track first (it is small and fast to decode), then
//! video frames a few ahead of display.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};
use std::thread::JoinHandle;

use anyhow::{Context, Result};

use crate::asf::{Asf, AudioFormat, MediaObject, VideoFormat};
use crate::vc1::{Picture, Vc1Decoder};
use crate::wma::WmaDecoder;

/// Decoded frames waiting ahead of display.
const FRAMES_AHEAD: usize = 4;

/// The movie's sound, interleaved, starting at the movie's first audio presentation time.
#[derive(Debug, Clone)]
pub struct AudioTrack {
    pub rate: u32,
    pub channels: u16,
    pub pcm: Vec<f32>,
    pub start_ms: i64,
}

enum Msg {
    Audio(AudioTrack),
    Frame(usize, Box<Picture>),
    Error(String),
}

/// One movie being decoded.
pub struct MovieStream {
    /// Presentation time of each video frame (milliseconds).
    pub frame_times_ms: Vec<i64>,
    /// Visible picture size.
    pub display: (u32, u32),
    /// Decoded picture size (whole macroblocks).
    pub coded: (u32, u32),
    rx: Receiver<Msg>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    audio: Option<AudioTrack>,
    audio_received: bool,
    current: Option<(usize, Box<Picture>)>,
    next: Option<(usize, Box<Picture>)>,
    error: Option<String>,
}

impl std::fmt::Debug for MovieStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MovieStream")
            .field("frames", &self.frame_times_ms.len())
            .field("display", &self.display)
            .field("current", &self.current.as_ref().map(|c| c.0))
            .finish()
    }
}

struct Job {
    audio: Vec<MediaObject>,
    video: Vec<MediaObject>,
    afmt: AudioFormat,
    vfmt: VideoFormat,
}

fn worker(job: Job, tx: SyncSender<Msg>, stop: Arc<AtomicBool>) {
    let Job { audio, video, afmt: fmt, vfmt } = job;
    let run = || -> Result<()> {
        let mut dec = WmaDecoder::new(&fmt)?;
        let mut pcm = Vec::new();
        for o in &audio {
            for sf in o.data.chunks(fmt.block_align as usize) {
                pcm.extend(dec.decode(sf)?);
            }
        }
        // The decoder's output starts with its own delay, which is not part of the sound.
        let skip = (dec.delay() * dec.channels()).min(pcm.len());
        pcm.drain(..skip);
        let start_ms = audio.first().map_or(0, |o| o.time_ms);
        let track = AudioTrack { rate: fmt.rate, channels: dec.channels() as u16, pcm, start_ms };
        if tx.send(Msg::Audio(track)).is_err() {
            return Ok(());
        }
        let mut vdec = Vc1Decoder::new(&vfmt)?;
        for (i, o) in video.iter().enumerate() {
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            let pic = vdec.decode(&o.data).with_context(|| format!("video frame {i}"))?;
            if tx.send(Msg::Frame(i, Box::new(pic.clone()))).is_err() {
                return Ok(());
            }
        }
        Ok(())
    };
    if let Err(e) = run() {
        let _ = tx.send(Msg::Error(format!("{e:#}")));
    }
}

impl MovieStream {
    /// Start decoding a movie file (an ASF container with WMV3 video and WMA v2 sound).
    pub fn open(file: &[u8]) -> Result<Self> {
        let asf = Asf::parse(file)?;
        let (vnum, vfmt) = asf.video.first().context("movie without video")?;
        let (vnum, display) = (*vnum, (vfmt.width, vfmt.height));
        let vfmt = vfmt.clone();
        let (anum, afmt) = asf.audio.first().context("movie without sound")?.clone();
        let (mut audio, mut video) = (Vec::new(), Vec::new());
        for o in asf.objects()? {
            if o.stream == vnum {
                video.push(o);
            } else if o.stream == anum {
                audio.push(o);
            }
        }
        let frame_times_ms = video.iter().map(|o| o.time_ms).collect();
        let coded = (display.0.div_ceil(16) * 16, display.1.div_ceil(16) * 16);
        let (tx, rx) = sync_channel(FRAMES_AHEAD);
        let stop = Arc::new(AtomicBool::new(false));
        let st = stop.clone();
        let thread = std::thread::Builder::new()
            .name("movie decode".into())
            .spawn(move || worker(Job { audio, video, afmt, vfmt }, tx, st))?;
        Ok(Self {
            frame_times_ms,
            display,
            coded,
            rx,
            stop,
            thread: Some(thread),
            audio: None,
            audio_received: false,
            current: None,
            next: None,
            error: None,
        })
    }

    fn receive(&mut self) {
        while self.next.is_none() {
            match self.rx.try_recv() {
                Ok(Msg::Audio(a)) => {
                    self.audio = Some(a);
                    self.audio_received = true;
                }
                Ok(Msg::Frame(i, p)) => self.next = Some((i, p)),
                Ok(Msg::Error(e)) => self.error = Some(e),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
    }

    /// The sound track, once decoded (taken by the first call that finds it).
    pub fn take_audio(&mut self) -> Option<AudioTrack> {
        self.receive();
        self.audio.take()
    }

    /// The sound is decoded and the first picture is ready.
    pub fn prepared(&mut self) -> bool {
        self.receive();
        self.audio_received && (self.current.is_some() || self.next.is_some())
    }

    /// A decoding error, if decoding stopped on one.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Advance to the last frame due at `t_ms` (as far as decoding has got) and return it with its index.
    pub fn frame_at(&mut self, t_ms: i64) -> Option<(usize, &Picture)> {
        loop {
            self.receive();
            match &self.next {
                Some((i, _)) if self.current.is_none() || self.frame_times_ms[*i] <= t_ms => {
                    self.current = self.next.take();
                }
                _ => break,
            }
        }
        self.current.as_ref().map(|(i, p)| (*i, &**p))
    }

    /// Index of the frame shown now, if any.
    pub fn current_index(&self) -> Option<usize> {
        self.current.as_ref().map(|c| c.0)
    }

    /// The last frame has been shown.
    pub fn finished(&self) -> bool {
        self.current_index().is_some_and(|i| i + 1 >= self.frame_times_ms.len()) || self.error.is_some()
    }
}

impl Drop for MovieStream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Unblock the decoder if it waits on a full queue.
        while self.rx.try_recv().is_ok() {}
        if let Some(t) = self.thread.take() {
            drop(std::mem::replace(&mut self.rx, sync_channel(0).1));
            let _ = t.join();
        }
    }
}
