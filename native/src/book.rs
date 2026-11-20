// Chapters, bookmarks and the sleep timer: the rules, kept apart from the UI so they can be tested. Same behaviour as the Tauri app.

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Chapter {
    pub start: f64,
    pub end: f64,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bookmark {
    pub time: f64,
    pub title: String,
}

/// The chapter playing at `t`, if the book has chapters.
pub fn chapter_at(chapters: &[Chapter], t: f64) -> Option<usize> {
    if chapters.is_empty() {
        return None;
    }
    Some(chapters.iter().rposition(|c| c.start <= t + 0.05).unwrap_or(0))
}

/// Where Next goes: the start of the next chapter, or nowhere on the last one.
pub fn next_start(chapters: &[Chapter], t: f64) -> Option<f64> {
    chapters.get(chapter_at(chapters, t)? + 1).map(|c| c.start)
}

/// Where Previous goes: back to the start of this chapter when more than 3 seconds in, otherwise to the previous chapter.
pub fn prev_start(chapters: &[Chapter], t: f64) -> Option<f64> {
    let i = chapter_at(chapters, t)?;
    let here = chapters[i].start;
    Some(if t - here > 3.0 || i == 0 { here } else { chapters[i - 1].start })
}

/// A bookmark's name: the chapter (or the book) and the time.
pub fn bookmark_title(chapters: &[Chapter], book: &str, t: f64, clock: impl Fn(f64) -> String) -> String {
    let place = chapter_at(chapters, t).map_or(book, |i| chapters[i].title.as_str());
    format!("{place} · {}", clock(t))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sleep {
    Off,
    /// pause at this wall-clock time, in seconds since the timer's clock started
    At(f64),
    /// pause when chapter `n` ends
    ChapterEnd(usize),
}

impl Sleep {
    /// True when playback should pause now. `now` is on the same clock as `At`; `t` is the book position.
    pub fn due(self, now: f64, chapters: &[Chapter], t: f64) -> bool {
        match self {
            Sleep::Off => false,
            Sleep::At(at) => now >= at,
            Sleep::ChapterEnd(n) => chapters.get(n).is_some_and(|c| t >= c.end - 0.3),
        }
    }

    /// Real seconds until the timer pauses (book time runs `speed` times faster), or None when it is off.
    pub fn left(self, now: f64, chapters: &[Chapter], t: f64, speed: f64) -> Option<f64> {
        match self {
            Sleep::Off => None,
            Sleep::At(at) => Some(at - now),
            Sleep::ChapterEnd(n) => chapters.get(n).map(|c| (c.end - t) / speed.max(0.1)),
        }
    }

    /// Ten more minutes: a timer moves out, and "end of chapter" becomes ten minutes after this chapter ends.
    pub fn extended(self, now: f64, chapters: &[Chapter], t: f64, speed: f64) -> Sleep {
        match self.left(now, chapters, t, speed) {
            Some(left) => Sleep::At(now + left.max(0.0) + EXTEND),
            None => Sleep::Off,
        }
    }
}

pub const EXTEND: f64 = 10.0 * 60.0;
/// The volume fades out over the timer's last 30 seconds, so it doesn't cut off mid-sentence.
pub const FADE: f64 = 30.0;

/// How loud to play with this much of the timer left: 1 until the fade, then down to 0.
pub fn fade(left: Option<f64>) -> f32 {
    left.map_or(1.0, |l| (l / FADE).clamp(0.0, 1.0) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> Vec<Chapter> {
        (0..3).map(|i| Chapter { start: i as f64 * 100.0, end: (i + 1) as f64 * 100.0, title: format!("Chapter {}", i + 1) }).collect()
    }

    #[test]
    fn finds_chapters() {
        let c = book();
        assert_eq!(chapter_at(&c, 0.0), Some(0));
        assert_eq!(chapter_at(&c, 150.0), Some(1));
        assert_eq!(chapter_at(&c, 99.97), Some(1)); // a hair before a start counts as that chapter, so a seek to it lands there
        assert_eq!(chapter_at(&[], 10.0), None);
    }

    #[test]
    fn next_and_previous() {
        let c = book();
        assert_eq!(next_start(&c, 50.0), Some(100.0));
        assert_eq!(next_start(&c, 250.0), None);
        assert_eq!(prev_start(&c, 150.0), Some(100.0)); // well into chapter 2: back to its start
        assert_eq!(prev_start(&c, 101.0), Some(0.0)); // just started chapter 2: to chapter 1
        assert_eq!(prev_start(&c, 1.0), Some(0.0)); // first chapter stays put
    }

    #[test]
    fn bookmark_names() {
        let clock = |t: f64| format!("{}s", t as u64);
        assert_eq!(bookmark_title(&book(), "Dune", 150.0, clock), "Chapter 2 · 150s");
        assert_eq!(bookmark_title(&[], "Dune", 5.0, clock), "Dune · 5s");
    }

    #[test]
    fn sleep_timer() {
        let c = book();
        assert!(!Sleep::Off.due(1e9, &c, 0.0));
        assert!(!Sleep::At(60.0).due(59.0, &c, 0.0));
        assert!(Sleep::At(60.0).due(60.0, &c, 0.0));
        assert!(!Sleep::ChapterEnd(0).due(0.0, &c, 99.0));
        assert!(Sleep::ChapterEnd(0).due(0.0, &c, 99.8));
        assert!(Sleep::ChapterEnd(0).due(0.0, &c, 120.0)); // a seek past the end still stops
    }

    #[test]
    fn sleep_fades_and_extends() {
        let c = book();
        assert_eq!(fade(Sleep::Off.left(0.0, &c, 0.0, 1.0)), 1.0);
        assert_eq!(fade(Sleep::At(100.0).left(0.0, &c, 0.0, 1.0)), 1.0);
        assert_eq!(fade(Sleep::At(100.0).left(85.0, &c, 0.0, 1.0)), 0.5);
        assert_eq!(fade(Sleep::At(100.0).left(120.0, &c, 0.0, 1.0)), 0.0);
        assert_eq!(Sleep::ChapterEnd(0).left(0.0, &c, 40.0, 2.0), Some(30.0)); // 60 s of book at 2×
        assert_eq!(Sleep::At(100.0).extended(40.0, &c, 0.0, 1.0), Sleep::At(700.0));
        assert_eq!(Sleep::ChapterEnd(0).extended(10.0, &c, 40.0, 1.0), Sleep::At(10.0 + 60.0 + EXTEND));
        assert_eq!(Sleep::Off.extended(0.0, &c, 0.0, 1.0), Sleep::Off);
    }
}
