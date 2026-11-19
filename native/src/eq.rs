// The equalizer: five filters run on every sample on its way to the speakers (a low shelf at 100 Hz, peaks at 300 Hz, 1 kHz and
// 3 kHz, a high shelf at 8 kHz), the same bands and presets as the Tauri app, which used the browser's BiquadFilter. The formulas are
// the standard "Audio EQ Cookbook" ones that the browser uses too.
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::Mutex;

pub const BANDS: [f32; 5] = [100.0, 300.0, 1000.0, 3000.0, 8000.0];
pub const PRESETS: [(&str, [f32; 5]); 5] = [
    ("Flat", [0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Voice", [-4.0, -1.0, 2.0, 4.0, 2.0]),
    ("Less bass", [-8.0, -4.0, 0.0, 0.0, 0.0]),
    ("More bass", [6.0, 3.0, 0.0, 0.0, 0.0]),
    ("Treble", [0.0, 0.0, 0.0, 3.0, 6.0]),
];

/// The gains in dB, shared between the UI and the audio thread; the version tells the audio thread to recompute its filters.
#[derive(Default)]
pub struct Gains {
    db: Mutex<[f32; 5]>,
    version: AtomicU64,
}

impl Gains {
    pub fn set(&self, db: [f32; 5]) {
        *self.db.lock().unwrap() = db;
        self.version.fetch_add(1, Relaxed);
    }
    pub fn get(&self) -> [f32; 5] {
        *self.db.lock().unwrap()
    }
    pub fn version(&self) -> u64 {
        self.version.load(Relaxed)
    }
}

#[derive(Clone, Copy, Default)]
struct Coef {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

fn coef(kind: usize, f: f32, db: f32, rate: f32) -> Coef {
    let a = 10f32.powf(db / 40.0);
    let w = std::f32::consts::TAU * f / rate;
    let (cos, sin) = (w.cos(), w.sin());
    let (b0, b1, b2, a0, a1, a2) = match kind {
        0 => {
            // low shelf, slope 1
            let al = sin / 2.0 * 2f32.sqrt();
            let k = 2.0 * a.sqrt() * al;
            (a * ((a + 1.0) - (a - 1.0) * cos + k), 2.0 * a * ((a - 1.0) - (a + 1.0) * cos), a * ((a + 1.0) - (a - 1.0) * cos - k),
             (a + 1.0) + (a - 1.0) * cos + k, -2.0 * ((a - 1.0) + (a + 1.0) * cos), (a + 1.0) + (a - 1.0) * cos - k)
        }
        4 => {
            // high shelf, slope 1
            let al = sin / 2.0 * 2f32.sqrt();
            let k = 2.0 * a.sqrt() * al;
            (a * ((a + 1.0) + (a - 1.0) * cos + k), -2.0 * a * ((a - 1.0) + (a + 1.0) * cos), a * ((a + 1.0) + (a - 1.0) * cos - k),
             (a + 1.0) - (a - 1.0) * cos + k, 2.0 * ((a - 1.0) - (a + 1.0) * cos), (a + 1.0) - (a - 1.0) * cos - k)
        }
        _ => {
            // peak, Q 1 (the browser's default)
            let al = sin / 2.0;
            (1.0 + al * a, -2.0 * cos, 1.0 - al * a, 1.0 + al / a, -2.0 * cos, 1.0 - al / a)
        }
    };
    Coef { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
}

/// The filters for one stream: call `process` on each interleaved sample in order.
pub struct Eq {
    rate: f32,
    ch: usize,
    flat: bool,
    coefs: [Coef; 5],
    state: Vec<[[f32; 4]; 5]>, // per channel, per band: x1, x2, y1, y2
    at: usize,                 // the channel of the next sample
}

impl Eq {
    pub fn new(rate: u32, channels: usize) -> Self {
        Eq { rate: rate as f32, ch: channels.max(1), flat: true, coefs: [Coef::default(); 5], state: vec![[[0.0; 4]; 5]; channels.max(1)], at: 0 }
    }

    pub fn set(&mut self, db: [f32; 5]) {
        self.flat = db.iter().all(|g| g.abs() < 0.01);
        let nyquist = self.rate / 2.0;
        for (i, (f, g)) in BANDS.iter().zip(db).enumerate() {
            self.coefs[i] = coef(i, f.min(nyquist * 0.95), g, self.rate);
        }
    }

    pub fn process(&mut self, x: f32) -> f32 {
        let c = self.at;
        self.at = (self.at + 1) % self.ch;
        if self.flat {
            return x;
        }
        let mut v = x;
        for (k, s) in self.coefs.iter().zip(self.state[c].iter_mut()) {
            let y = k.b0 * v + k.b1 * s[0] + k.b2 * s[1] - k.a1 * s[2] - k.a2 * s[3];
            (s[1], s[0], s[3], s[2]) = (s[0], v, s[2], y);
            v = y;
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // the steady-state level of a sine after the equalizer, relative to before
    fn gain_at(freq: f32, db: [f32; 5]) -> f32 {
        let rate = 44100;
        let mut eq = Eq::new(rate, 1);
        eq.set(db);
        let n = rate as usize;
        let out: Vec<f32> = (0..n).map(|i| eq.process((std::f32::consts::TAU * freq * i as f32 / rate as f32).sin())).collect();
        out[n / 2..].iter().fold(0f32, |m, v| m.max(v.abs()))
    }

    #[test]
    fn flat_changes_nothing() {
        let mut eq = Eq::new(44100, 2);
        eq.set([0.0; 5]);
        assert_eq!(eq.process(0.3), 0.3);
    }

    #[test]
    fn bands_boost_and_cut_where_they_should() {
        let db = |g: f32| 10f32.powf(g / 20.0);
        // +12 dB bass shelf: 40 Hz rises about 12 dB, 5 kHz stays put
        let bass = [12.0, 0.0, 0.0, 0.0, 0.0];
        assert!((gain_at(40.0, bass) - db(12.0)).abs() < 0.6, "{}", gain_at(40.0, bass));
        assert!((gain_at(5000.0, bass) - 1.0).abs() < 0.05);
        // -6 dB at 1 kHz: that tone drops to half, 100 Hz barely moves
        let mid = [0.0, 0.0, -6.0, 0.0, 0.0];
        assert!((gain_at(1000.0, mid) - db(-6.0)).abs() < 0.03, "{}", gain_at(1000.0, mid));
        assert!((gain_at(100.0, mid) - 1.0).abs() < 0.05);
        // +6 dB treble shelf lifts 15 kHz
        assert!((gain_at(15000.0, [0.0, 0.0, 0.0, 0.0, 6.0]) - db(6.0)).abs() < 0.2);
    }
}
