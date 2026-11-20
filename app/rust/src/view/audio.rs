//! In-app audio: the file is decoded from memory by Symphonia and played
//! straight to the sound device by cpal. No temporary file and no other app
//! ever sees the audio.
use std::collections::VecDeque;
use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::{MetadataOptions, StandardTagKey};
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;
use zeroize::{Zeroize, Zeroizing};

use crate::error::{CoreError, CoreResult};

pub const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "m4a", "m4b", "aac", "mp4", "flac", "ogg", "oga", "wav", "wave", "aif", "aiff", "caf",
];

/// The file's bytes, shared by the probe and the player and wiped when the
/// last of them lets go.
type Bytes = Arc<Zeroizing<Vec<u8>>>;

struct Mem {
    data: Bytes,
    pos: u64,
}

impl Read for Mem {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let start = (self.pos as usize).min(self.data.len());
        let n = buf.len().min(self.data.len() - start);
        buf[..n].copy_from_slice(&self.data[start..start + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for Mem {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let len = self.data.len() as i128;
        let next = match to {
            SeekFrom::Start(n) => n as i128,
            SeekFrom::End(n) => len + n as i128,
            SeekFrom::Current(n) => self.pos as i128 + n as i128,
        };
        if next < 0 {
            return Err(std::io::Error::other("seek before start"));
        }
        self.pos = next as u64;
        Ok(self.pos)
    }
}

impl MediaSource for Mem {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.data.len() as u64)
    }
}

fn bad_audio() -> CoreError {
    CoreError::InvalidInput {
        message: "this audio file could not be read, or its format is not supported".into(),
    }
}

struct Track {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    id: u32,
    rate: u32,
    channels: usize,
}

fn open_track(data: &Bytes, ext: &str) -> CoreResult<(Track, Info)> {
    let src = Mem {
        data: data.clone(),
        pos: 0,
    };
    let mss = MediaSourceStream::new(Box::new(src), Default::default());
    let mut hint = Hint::new();
    if !ext.is_empty() {
        hint.with_extension(ext);
    }
    let mut probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions {
                enable_gapless: true,
                ..Default::default()
            },
            &MetadataOptions::default(),
        )
        .map_err(|_| bad_audio())?;
    let mut info = Info::default();
    let mut read_tags = |rev: Option<&symphonia::core::meta::MetadataRevision>| {
        for tag in rev.map(|r| r.tags()).unwrap_or_default() {
            match tag.std_key {
                Some(StandardTagKey::TrackTitle) if info.title.is_empty() => {
                    info.title = tag.value.to_string()
                }
                Some(StandardTagKey::Artist) if info.artist.is_empty() => {
                    info.artist = tag.value.to_string()
                }
                Some(StandardTagKey::Album) if info.album.is_empty() => {
                    info.album = tag.value.to_string()
                }
                _ => {}
            }
        }
    };
    if let Some(m) = probed.metadata.get() {
        read_tags(m.current());
    }
    let mut format = probed.format;
    read_tags(format.metadata().current());
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(bad_audio)?;
    let params = track.codec_params.clone();
    let rate = params.sample_rate.ok_or_else(bad_audio)?;
    let channels = params.channels.map(|c| c.count()).unwrap_or(2).max(1);
    if let (Some(frames), Some(tb)) = (params.n_frames, params.time_base) {
        let t = tb.calc_time(frames);
        info.duration_ms = t.seconds * 1000 + (t.frac * 1000.0) as u64;
    } else if let Some(frames) = params.n_frames {
        info.duration_ms = frames * 1000 / rate as u64;
    }
    info.sample_rate = rate;
    info.channels = channels as u32;
    let id = track.id;
    let decoder = symphonia::default::get_codecs()
        .make(&params, &DecoderOptions::default())
        .map_err(|_| bad_audio())?;
    Ok((
        Track {
            format,
            decoder,
            id,
            rate,
            channels,
        },
        info,
    ))
}

#[derive(Clone, Debug, Default)]
pub struct Info {
    pub duration_ms: u64,
    pub sample_rate: u32,
    pub channels: u32,
    pub title: String,
    pub artist: String,
    pub album: String,
}

#[derive(Clone, Debug, Default)]
pub struct State {
    pub position_ms: u64,
    pub playing: bool,
    pub ended: bool,
    /// Set when the sound device failed.
    pub error: Option<String>,
}

enum Cmd {
    Play,
    Pause,
    Seek(u64),
    Stop,
}

#[derive(Default)]
struct Shared {
    /// Output frames played since the last seek.
    played: AtomicU64,
    /// Where the last seek landed.
    base_ms: AtomicU64,
    playing: AtomicBool,
    ended: AtomicBool,
    error: Mutex<Option<String>>,
}

/// An open audio file. Playback starts on the first [Audio::play].
pub struct Audio {
    data: Bytes,
    ext: String,
    info: Info,
    player: Mutex<Option<Player>>,
}

struct Player {
    cmd: mpsc::Sender<Cmd>,
    shared: Arc<Shared>,
    out_rate: u32,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.cmd.send(Cmd::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Audio {
    pub fn open(name: &str, data: Zeroizing<Vec<u8>>) -> CoreResult<Audio> {
        let ext = name
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        let data: Bytes = Arc::new(data);
        let (_, info) = open_track(&data, &ext)?;
        Ok(Audio {
            data,
            ext,
            info,
            player: Mutex::new(None),
        })
    }

    pub fn info(&self) -> &Info {
        &self.info
    }

    fn player(&self) -> std::sync::MutexGuard<'_, Option<Player>> {
        self.player.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn send(&self, cmd: Cmd) -> CoreResult<()> {
        let mut g = self.player();
        if g.is_none() {
            *g = Some(start(self.data.clone(), &self.ext)?);
        }
        let p = g.as_ref().ok_or_else(bad_audio)?;
        p.cmd.send(cmd).map_err(|_| no_device("the player stopped"))
    }

    pub fn play(&self) -> CoreResult<()> {
        self.send(Cmd::Play)
    }

    pub fn pause(&self) {
        if let Some(p) = self.player().as_ref() {
            let _ = p.cmd.send(Cmd::Pause);
        }
    }

    pub fn seek(&self, ms: u64) -> CoreResult<()> {
        let ms = ms.min(self.info.duration_ms);
        if self.player().is_none() {
            // Nothing to move yet; open the player so the position sticks.
            self.send(Cmd::Pause)?;
        }
        self.send(Cmd::Seek(ms))
    }

    pub fn state(&self) -> State {
        let g = self.player();
        let Some(p) = g.as_ref() else {
            return State::default();
        };
        let s = &p.shared;
        let ended = s.ended.load(Ordering::Relaxed);
        let pos = s.base_ms.load(Ordering::Relaxed)
            + s.played.load(Ordering::Relaxed) * 1000 / p.out_rate.max(1) as u64;
        let state = State {
            position_ms: if ended {
                self.info.duration_ms.max(pos)
            } else {
                pos
            },
            playing: s.playing.load(Ordering::Relaxed),
            ended,
            error: s.error.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        };
        state
    }
}

fn no_device(why: &str) -> CoreError {
    CoreError::InvalidInput {
        message: format!("audio could not play: {why}"),
    }
}

/// Converts decoded frames to the device's channel count and sample rate
/// (linear interpolation, which is plenty for listening).
struct Converter {
    in_ch: usize,
    out_ch: usize,
    /// in rate / out rate
    step: f64,
    pos: f64,
    /// Input frames (already mapped to `out_ch`) not yet used.
    pending: VecDeque<f32>,
}

impl Converter {
    fn new(in_ch: usize, in_rate: u32, out_ch: usize, out_rate: u32) -> Self {
        Converter {
            in_ch,
            out_ch,
            step: in_rate as f64 / out_rate as f64,
            pos: 0.0,
            pending: VecDeque::new(),
        }
    }

    fn reset(&mut self) {
        self.pos = 0.0;
        wipe(&mut self.pending);
    }

    fn push(&mut self, input: &[f32], out: &mut VecDeque<f32>) {
        let (ic, oc) = (self.in_ch, self.out_ch);
        for frame in input.chunks(ic) {
            for c in 0..oc {
                let v = if oc == 1 {
                    frame.iter().sum::<f32>() / frame.len().max(1) as f32
                } else {
                    frame
                        .get(c)
                        .or_else(|| frame.get(c % ic))
                        .copied()
                        .unwrap_or(0.0)
                };
                self.pending.push_back(v);
            }
        }
        if (self.step - 1.0).abs() < 1e-9 {
            out.extend(self.pending.drain(..));
            return;
        }
        let frames = self.pending.len() / oc;
        while self.pos + 1.0 < frames as f64 {
            let i = self.pos as usize;
            let t = (self.pos - i as f64) as f32;
            for c in 0..oc {
                let a = self.pending[i * oc + c];
                let b = self.pending[(i + 1) * oc + c];
                out.push_back(a + (b - a) * t);
            }
            self.pos += self.step;
        }
        let used = (self.pos as usize).min(frames);
        self.pending.drain(..used * oc);
        self.pos -= used as f64;
    }
}

fn wipe(q: &mut VecDeque<f32>) {
    let (a, b) = q.as_mut_slices();
    a.zeroize();
    b.zeroize();
    q.clear();
}

/// Starts the player thread, which owns the sound device stream (not all
/// platforms let a stream move between threads).
fn start(data: Bytes, ext: &str) -> CoreResult<Player> {
    let (track, _) = open_track(&data, ext)?;
    drop(data);
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::channel::<CoreResult<u32>>();
    let shared = Arc::new(Shared::default());
    let sh = shared.clone();
    let thread = std::thread::Builder::new()
        .name("audio".into())
        .spawn(move || run(track, cmd_rx, ready_tx, sh))
        .map_err(|_| no_device("no thread"))?;
    let out_rate = ready_rx
        .recv()
        .map_err(|_| no_device("the sound device did not start"))??;
    Ok(Player {
        cmd: cmd_tx,
        shared,
        out_rate,
        thread: Some(thread),
    })
}

type Ring = Arc<Mutex<VecDeque<f32>>>;

fn build_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    ring: Ring,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    use cpal::traits::DeviceTrait;
    let ch = config.channels as usize;
    let err_shared = shared.clone();
    device.build_output_stream(
        config,
        move |out: &mut [T], _: &cpal::OutputCallbackInfo| {
            let playing = shared.playing.load(Ordering::Relaxed);
            let mut q = ring.lock().unwrap_or_else(|e| e.into_inner());
            let mut frames = 0u64;
            for frame in out.chunks_mut(ch) {
                let have = playing && q.len() >= ch;
                for s in frame.iter_mut() {
                    let v = if have {
                        q.pop_front().unwrap_or(0.0)
                    } else {
                        0.0
                    };
                    *s = T::from_sample(v);
                }
                if have {
                    frames += 1;
                }
            }
            shared.played.fetch_add(frames, Ordering::Relaxed);
        },
        move |e: cpal::Error| {
            *err_shared.error.lock().unwrap_or_else(|p| p.into_inner()) = Some(e.to_string());
        },
        None,
    )
}

fn open_device(
    in_rate: u32,
    in_ch: usize,
    ring: Ring,
    shared: Arc<Shared>,
) -> CoreResult<(cpal::Stream, u32, usize)> {
    use cpal::traits::{DeviceTrait, HostTrait};
    use cpal::SampleFormat as F;
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| no_device("no sound output found"))?;
    // Prefer the file's own rate and channel count so nothing is converted.
    let exact = device.supported_output_configs().ok().and_then(|cs| {
        cs.filter(|c| {
            c.channels() as usize == in_ch.min(2)
                && c.min_sample_rate() <= in_rate
                && in_rate <= c.max_sample_rate()
        })
        .max_by_key(|c| matches!(c.sample_format(), F::F32) as u8)
        .map(|c| c.with_sample_rate(in_rate))
    });
    let supported = match exact {
        Some(c) => c,
        None => device
            .default_output_config()
            .map_err(|e| no_device(&e.to_string()))?,
    };
    let config: cpal::StreamConfig = supported.config();
    let (rate, ch) = (config.sample_rate, config.channels as usize);
    let stream = match supported.sample_format() {
        F::F32 => build_stream::<f32>(&device, config, ring, shared),
        F::I16 => build_stream::<i16>(&device, config, ring, shared),
        F::U16 => build_stream::<u16>(&device, config, ring, shared),
        F::I32 => build_stream::<i32>(&device, config, ring, shared),
        F::U8 => build_stream::<u8>(&device, config, ring, shared),
        F::F64 => build_stream::<f64>(&device, config, ring, shared),
        _ => return Err(no_device("the sound output's format is not supported")),
    }
    .map_err(|e| no_device(&e.to_string()))?;
    Ok((stream, rate, ch))
}

fn run(
    mut t: Track,
    cmd: mpsc::Receiver<Cmd>,
    ready: mpsc::Sender<CoreResult<u32>>,
    shared: Arc<Shared>,
) {
    use cpal::traits::StreamTrait;
    let ring: Ring = Arc::new(Mutex::new(VecDeque::new()));
    let (stream, out_rate, out_ch) =
        match open_device(t.rate, t.channels, ring.clone(), shared.clone()) {
            Ok(s) => s,
            Err(e) => {
                let _ = ready.send(Err(e));
                return;
            }
        };
    if let Err(e) = stream.play() {
        let _ = ready.send(Err(no_device(&e.to_string())));
        return;
    }
    let _ = ready.send(Ok(out_rate));
    let mut conv = Converter::new(t.channels, t.rate, out_ch, out_rate);
    // About half a second of sound queued ahead.
    let target = out_rate as usize * out_ch / 2;
    let mut eof = false;
    let mut staged: VecDeque<f32> = VecDeque::new();
    let seek = |t: &mut Track, conv: &mut Converter, ms: u64| {
        let to = SeekTo::Time {
            time: Time::new(ms / 1000, (ms % 1000) as f64 / 1000.0),
            track_id: Some(t.id),
        };
        let landed = match t.format.seek(SeekMode::Accurate, to) {
            Ok(s) => {
                let tb = t
                    .format
                    .tracks()
                    .iter()
                    .find(|x| x.id == t.id)
                    .and_then(|x| x.codec_params.time_base);
                tb.map(|tb| {
                    let tm = tb.calc_time(s.actual_ts);
                    tm.seconds * 1000 + (tm.frac * 1000.0) as u64
                })
                .unwrap_or(ms)
            }
            Err(_) => ms,
        };
        t.decoder.reset();
        conv.reset();
        landed
    };
    loop {
        // Commands first; wait briefly when the queue is full or done.
        let queued = ring.lock().unwrap_or_else(|e| e.into_inner()).len();
        let idle = eof || queued >= target;
        let next = if idle {
            cmd.recv_timeout(Duration::from_millis(20))
        } else {
            cmd.try_recv().map_err(|e| match e {
                mpsc::TryRecvError::Empty => mpsc::RecvTimeoutError::Timeout,
                mpsc::TryRecvError::Disconnected => mpsc::RecvTimeoutError::Disconnected,
            })
        };
        match next {
            Ok(Cmd::Play) => {
                if shared.ended.load(Ordering::Relaxed) {
                    let landed = seek(&mut t, &mut conv, 0);
                    wipe(&mut ring.lock().unwrap_or_else(|e| e.into_inner()));
                    shared.base_ms.store(landed, Ordering::Relaxed);
                    shared.played.store(0, Ordering::Relaxed);
                    shared.ended.store(false, Ordering::Relaxed);
                    eof = false;
                }
                shared.playing.store(true, Ordering::Relaxed);
            }
            Ok(Cmd::Pause) => shared.playing.store(false, Ordering::Relaxed),
            Ok(Cmd::Seek(ms)) => {
                let landed = seek(&mut t, &mut conv, ms);
                let mut q = ring.lock().unwrap_or_else(|e| e.into_inner());
                wipe(&mut q);
                shared.base_ms.store(landed, Ordering::Relaxed);
                shared.played.store(0, Ordering::Relaxed);
                drop(q);
                shared.ended.store(false, Ordering::Relaxed);
                eof = false;
            }
            Ok(Cmd::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if eof {
            if queued == 0 && shared.playing.load(Ordering::Relaxed) {
                shared.playing.store(false, Ordering::Relaxed);
                shared.ended.store(true, Ordering::Relaxed);
            }
            continue;
        }
        if queued >= target {
            continue;
        }
        // Decode one packet.
        let packet = match t.format.next_packet() {
            Ok(p) => p,
            Err(_) => {
                eof = true;
                continue;
            }
        };
        if packet.track_id() != t.id {
            continue;
        }
        match t.decoder.decode(&packet) {
            Ok(decoded) => {
                let mut sb = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
                sb.copy_interleaved_ref(decoded);
                conv.push(sb.samples(), &mut staged);
                sb.samples_mut().zeroize();
                let mut q = ring.lock().unwrap_or_else(|e| e.into_inner());
                q.extend(staged.drain(..));
            }
            // A damaged packet: skip it, as players do.
            Err(symphonia::core::errors::Error::DecodeError(_)) => {}
            Err(_) => eof = true,
        }
    }
    shared.playing.store(false, Ordering::Relaxed);
    drop(stream);
    wipe(&mut ring.lock().unwrap_or_else(|e| e.into_inner()));
    conv.reset();
}
