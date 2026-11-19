// The playback engine. A decode thread opens the book's audio files one after another (streamed with HttpSource), stretches them to the
// chosen speed and queues about two seconds of samples. The audio thread only reads from that queue (Out), so network stalls never reach
// it; when the queue runs dry it plays silence. Seeking and speed changes bump a generation number so already-queued audio is dropped.
use crate::api::Track;
use crate::http_source::HttpSource;
use crate::stretch::Stretch;
use rodio::{ChannelCount, Decoder, SampleRate, Source};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering::Relaxed};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const PLAYING: u8 = 1; // 0, the default, is loading
pub const ENDED: u8 = 2;
pub const FAILED: u8 = 3;

#[derive(Default)]
pub struct Status {
    pos: AtomicU64, // f64 bits: the book position of what is being heard
    epoch: AtomicU64,
    pub state: AtomicU8,
    pub stop: AtomicBool,
    pub error: Mutex<String>,
}

impl Status {
    pub fn pos(&self) -> f64 {
        f64::from_bits(self.pos.load(Relaxed))
    }
    fn set_pos(&self, t: f64) {
        self.pos.store(t.to_bits(), Relaxed);
    }
    fn fail(&self, e: impl ToString) {
        *self.error.lock().unwrap() = e.to_string();
        self.state.store(FAILED, Relaxed);
    }
}

enum Cmd {
    Seek(f64),
    Speed(f64),
}

struct Chunk {
    epoch: u64,
    pos: f64,
    speed: f64,
    data: Vec<f32>,
}

/// The handle the UI keeps for one book.
pub struct Engine {
    tx: Sender<Cmd>,
    pub status: Arc<Status>,
}

impl Engine {
    /// Starts decoding `tracks` at `at` seconds. `ready` gets the audio to hand to the output once the first file's format is known.
    pub fn spawn(http: reqwest::blocking::Client, token: String, tracks: Vec<Track>, at: f64, speed: f64, ready: impl FnOnce(Out) + Send + 'static) -> Engine {
        let (tx, rx) = mpsc::channel();
        let status = Arc::new(Status::default());
        status.set_pos(at);
        let st = status.clone();
        std::thread::spawn(move || {
            if let Err(e) = decode(http, token, tracks, at, speed, rx, st.clone(), ready) {
                st.fail(e);
            }
        });
        Engine { tx, status }
    }

    pub fn seek(&self, t: f64) {
        self.status.set_pos(t.max(0.0));
        let _ = self.tx.send(Cmd::Seek(t.max(0.0)));
    }

    pub fn set_speed(&self, s: f64) {
        let _ = self.tx.send(Cmd::Speed(s));
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.status.stop.store(true, Relaxed);
    }
}

fn open(http: &reqwest::blocking::Client, token: &str, t: &Track) -> Result<Decoder<HttpSource>, String> {
    let src = HttpSource::open(http.clone(), token.into(), t.url.clone()).map_err(|e| format!("Can't open the audio: {e}"))?;
    let (len, mime) = (src.len, src.mime.clone());
    let mut b = Decoder::builder().with_data(src).with_byte_len(len).with_coarse_seek(true);
    if let Some(m) = &mime {
        b = b.with_mime_type(m);
    }
    b.build().map_err(|e| format!("Can't decode this file: {e}"))
}

/// The file that holds book time `t`, and where in that file it is.
fn locate(tracks: &[Track], t: f64) -> (usize, f64) {
    let i = tracks.iter().rposition(|tr| tr.start <= t).unwrap_or(0);
    (i, (t - tracks[i].start).max(0.0))
}

/// Turns `n` frames of `from` channels into `to` channels: mono is copied to every channel, otherwise channels are averaged or dropped.
fn remix(data: &[f32], from: usize, to: usize, out: &mut Vec<f32>) {
    if from == to {
        out.extend_from_slice(data);
        return;
    }
    for f in data.chunks_exact(from) {
        if to == 1 {
            out.push(f.iter().sum::<f32>() / from as f32);
        } else {
            (0..to).for_each(|c| out.push(if from == 1 { f[0] } else { f.get(c).copied().unwrap_or(0.0) }));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn decode(http: reqwest::blocking::Client, token: String, tracks: Vec<Track>, at: f64, mut speed: f64, rx: Receiver<Cmd>, sta: Arc<Status>, ready: impl FnOnce(Out)) -> Result<(), String> {
    let st = &*sta;
    let (mut ti, off) = locate(&tracks, at);
    let mut dec = open(&http, &token, &tracks[ti])?;
    let _ = dec.try_seek(Duration::from_secs_f64(off));
    // the output keeps the first file's format for the whole book
    // shortcut: a later file with a different sample rate plays at the wrong pitch; resample (rubato) if a real book ever does this
    let (ch, rate) = (dec.channels(), dec.sample_rate());
    let (q_tx, q_rx) = mpsc::sync_channel::<Chunk>(24);
    ready(Out { rx: q_rx, st: sta.clone(), cur: vec![], idx: 0, epoch: 0, pos: at, speed, ch, rate });
    let (chn, hz) = (ch.get() as usize, rate.get() as f64);
    let mut stretch = Stretch::new(chn, rate.get());
    stretch.reset(speed, at * hz);
    st.state.store(PLAYING, Relaxed);
    let mut raw = Vec::with_capacity(4096 * 2);
    let mut mixed = Vec::with_capacity(4096 * 2);
    let mut pending: Option<Chunk> = None;
    let mut epoch = 0u64;

    let restart = |t: f64, speed: f64, ti: &mut usize, dec: &mut Decoder<HttpSource>, stretch: &mut Stretch, epoch: &mut u64| -> Result<(), String> {
        *epoch += 1;
        st.epoch.store(*epoch, Relaxed);
        st.set_pos(t);
        let (i, off) = locate(&tracks, t);
        if i != *ti {
            *dec = open(&http, &token, &tracks[i])?;
            *ti = i;
        }
        let _ = dec.try_seek(Duration::from_secs_f64(off));
        stretch.reset(speed, t * hz);
        if st.state.load(Relaxed) == ENDED {
            st.state.store(PLAYING, Relaxed);
        }
        Ok(())
    };

    while !st.stop.load(Relaxed) {
        // waiting: either the queue is full (or paused), or the book has ended
        let cmd = if pending.is_some() || st.state.load(Relaxed) == ENDED {
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(c) => Some(c),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
        } else {
            rx.try_recv().ok()
        };
        if let Some(c) = cmd {
            let t = match c {
                Cmd::Seek(t) => t,
                Cmd::Speed(s) => {
                    speed = s;
                    st.pos()
                }
            };
            restart(t, speed, &mut ti, &mut dec, &mut stretch, &mut epoch)?;
            pending = None;
            continue;
        }
        if let Some(c) = pending.take() {
            match q_tx.try_send(c) {
                Ok(()) => {}
                Err(TrySendError::Full(c)) => pending = Some(c),
                Err(TrySendError::Disconnected(_)) => return Ok(()),
            }
            continue;
        }
        if st.state.load(Relaxed) == ENDED {
            continue;
        }
        let dch = dec.channels().get() as usize;
        raw.clear();
        raw.extend(dec.by_ref().take(4096 * dch));
        if raw.is_empty() {
            // end of this file: on to the next, or the end of the book
            if ti + 1 < tracks.len() {
                ti += 1;
                dec = open(&http, &token, &tracks[ti])?;
            } else {
                st.state.store(ENDED, Relaxed);
            }
            continue;
        }
        mixed.clear();
        remix(&raw[..raw.len() - raw.len() % dch], dch, chn, &mut mixed);
        stretch.push(&mixed);
        let pos = stretch.played() / hz;
        let mut data = Vec::with_capacity(mixed.len() * 2);
        stretch.pull(&mut data);
        if !data.is_empty() {
            pending = Some(Chunk { epoch, pos, speed: stretch.speed(), data });
        }
    }
    Ok(())
}

/// What the audio device plays: samples from the decode thread's queue, silence while it is empty.
pub struct Out {
    rx: Receiver<Chunk>,
    st: Arc<Status>,
    cur: Vec<f32>,
    idx: usize,
    epoch: u64,
    pos: f64,
    speed: f64,
    ch: ChannelCount,
    rate: SampleRate,
}

impl Iterator for Out {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.st.stop.load(Relaxed) {
            return None;
        }
        loop {
            let epoch = self.st.epoch.load(Relaxed);
            if self.epoch == epoch && self.idx < self.cur.len() {
                let v = self.cur[self.idx];
                self.idx += 1;
                let ch = self.ch.get() as usize;
                if self.idx % (ch * 512) == 0 {
                    self.st.set_pos(self.pos + (self.idx / ch) as f64 / self.rate.get() as f64 * self.speed);
                }
                return Some(v);
            }
            match self.rx.try_recv() {
                Ok(c) if c.epoch == epoch => {
                    (self.cur, self.idx, self.epoch, self.pos, self.speed) = (c.data, 0, c.epoch, c.pos, c.speed);
                    self.st.set_pos(c.pos);
                }
                Ok(_) => {} // queued before a seek: drop it
                Err(mpsc::TryRecvError::Empty) => return Some(0.0),
                Err(mpsc::TryRecvError::Disconnected) => return None,
            }
        }
    }
}

impl Source for Out {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        self.ch
    }
    fn sample_rate(&self) -> SampleRate {
        self.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locate_finds_the_file() {
        let t = |start| Track { url: String::new(), start };
        let tracks = [t(0.0), t(100.0), t(200.0)];
        assert_eq!(locate(&tracks, 0.0), (0, 0.0));
        assert_eq!(locate(&tracks, 150.0), (1, 50.0));
        assert_eq!(locate(&tracks, 250.0), (2, 50.0));
    }

    #[test]
    fn remix_channels() {
        let mut out = vec![];
        remix(&[0.5, 0.25], 1, 2, &mut out);
        assert_eq!(out, vec![0.5, 0.5, 0.25, 0.25]);
        out.clear();
        remix(&[1.0, 0.0], 2, 1, &mut out);
        assert_eq!(out, vec![0.5]);
    }
}
