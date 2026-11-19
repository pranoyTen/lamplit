// Changes playback speed without changing pitch (WSOLA: waveform-similarity overlap-add). Audio is cut into overlapping 25 ms windows that
// are read further apart (faster) or closer together (slower) than they are written. Each window is nudged by up to ±10 ms to where its
// waveform best continues the previous one, so voices stay smooth. At 1× the audio passes straight through.
pub struct Stretch {
    ch: usize,
    hs: usize,     // output hop, in frames (half a window)
    n: usize,      // window length, in frames
    search: usize, // how far a window may be nudged, in frames
    speed: f64,
    win: Vec<f32>,
    inp: Vec<f32>,          // interleaved input not yet consumed
    ia: f64,                // next analysis position, in frames from the start of `inp`
    prev: Option<usize>,    // where the previous window started, in frames from the start of `inp`
    tail: Vec<f32>,         // the second half of the previous window, already weighted, waiting to be overlapped
    played: f64,            // input frames advanced so far: the position in the source of the next output frame
}

impl Stretch {
    pub fn new(channels: usize, rate: u32) -> Self {
        let hs = (rate as usize / 80).max(16);
        let n = hs * 2;
        let win = (0..n).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos()).collect();
        Stretch { ch: channels, hs, n, search: rate as usize / 100, speed: 1.0, win, inp: vec![], ia: 0.0, prev: None, tail: vec![0.0; hs * channels], played: 0.0 }
    }

    pub fn speed(&self) -> f64 {
        self.speed
    }

    /// Changes speed and drops what is buffered (the caller flushes its output too). `at` is the source position to count from.
    pub fn reset(&mut self, speed: f64, at: f64) {
        self.speed = speed;
        self.inp.clear();
        self.ia = 0.0;
        self.prev = None;
        self.tail.iter_mut().for_each(|s| *s = 0.0);
        self.played = at;
    }

    /// Source frames advanced so far; the next pulled frame starts here.
    pub fn played(&self) -> f64 {
        self.played
    }

    pub fn push(&mut self, input: &[f32]) {
        self.inp.extend_from_slice(input);
    }

    /// Appends as much stretched output as the buffered input allows.
    pub fn pull(&mut self, out: &mut Vec<f32>) {
        let ch = self.ch;
        if self.speed == 1.0 {
            self.played += (self.inp.len() / ch) as f64;
            out.append(&mut self.inp);
            return;
        }
        let (hs, n, search) = (self.hs, self.n, self.search as isize);
        loop {
            let avail = self.inp.len() / ch;
            let target = self.ia.round() as isize;
            if (target + search) as usize + n > avail || self.prev.is_some_and(|p| p + n > avail) {
                return;
            }
            let start = match self.prev {
                None => target.max(0) as usize,
                Some(p) => self.best((target - search).max(0) as usize, (target + search) as usize, p + hs),
            };
            for f in 0..hs {
                for c in 0..ch {
                    out.push(self.tail[f * ch + c] + self.inp[(start + f) * ch + c] * self.win[f]);
                }
            }
            for f in 0..hs {
                for c in 0..ch {
                    self.tail[f * ch + c] = self.inp[(start + hs + f) * ch + c] * self.win[hs + f];
                }
            }
            self.prev = Some(start);
            let ha = hs as f64 * self.speed;
            self.ia += ha;
            self.played += ha;
            // drop input that no later window can reach
            let keep = (start + hs).min((self.ia.round() as isize - search).max(0) as usize);
            self.inp.drain(..keep * ch);
            self.ia -= keep as f64;
            self.prev = Some(start - keep);
        }
    }

    // the start in lo..=hi whose first half-window best matches the natural continuation at `nat` (normalised cross-correlation of the
    // channel sum, every 4th frame to keep it cheap)
    fn best(&self, lo: usize, hi: usize, nat: usize) -> usize {
        let mono = |f: usize| (0..self.ch).map(|c| self.inp[f * self.ch + c]).sum::<f32>();
        let reference: Vec<f32> = (0..self.hs).step_by(4).map(|i| mono(nat + i)).collect();
        let (mut best, mut score) = (lo, f32::MIN);
        for s in lo..=hi {
            let (mut dot, mut energy) = (0.0f32, 1e-9f32);
            for (k, i) in (0..self.hs).step_by(4).enumerate() {
                let v = mono(s + i);
                dot += v * reference[k];
                energy += v * v;
            }
            let sc = dot / energy.sqrt();
            if sc > score {
                (best, score) = (s, sc);
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::Stretch;

    // a 220 Hz tone (about a low voice), stretched: the length scales with 1/speed and the pitch (zero crossings per second) stays put
    fn check(speed: f64) {
        let rate = 44100;
        let input: Vec<f32> = (0..rate * 2).map(|i| (std::f32::consts::TAU * 220.0 * i as f32 / rate as f32).sin() * 0.5).collect();
        let mut s = Stretch::new(1, rate as u32);
        s.reset(speed, 0.0);
        let mut out = vec![];
        for chunk in input.chunks(1000) {
            s.push(chunk);
            s.pull(&mut out);
        }
        let want = input.len() as f64 / speed;
        assert!((out.len() as f64 - want).abs() < want * 0.04, "speed {speed}: {} frames, wanted about {want}", out.len());
        let body = &out[2000..out.len() - 2000];
        let crossings = body.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count() as f64;
        let hz = crossings / 2.0 / (body.len() as f64 / rate as f64);
        assert!((hz - 220.0).abs() < 220.0 * 0.03, "speed {speed}: pitch {hz:.1} Hz");
        assert!((s.played() - (input.len() as f64)).abs() < rate as f64 * 0.05, "speed {speed}: played {}", s.played());
    }

    #[test]
    fn faster_keeps_pitch() {
        check(1.5);
        check(2.0);
    }

    #[test]
    fn slower_keeps_pitch() {
        check(0.75);
    }

    #[test]
    fn normal_speed_passes_through() {
        let mut s = Stretch::new(2, 44100);
        s.push(&[0.1, 0.2, 0.3, 0.4]);
        let mut out = vec![];
        s.pull(&mut out);
        assert_eq!(out, vec![0.1, 0.2, 0.3, 0.4]);
        assert_eq!(s.played(), 2.0);
    }
}
