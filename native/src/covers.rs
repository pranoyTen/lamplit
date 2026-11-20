// Cover images: fetched and decoded by a few worker threads, scaled to thumbnails, and kept in a small cache so memory stays bounded.
// Requests are served newest first, so after a fast scroll the covers now on screen come before the ones scrolled past.
use crate::api::Client;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::{Arc, Condvar, Mutex};

// covers are decoded at the size the screen draws them, so they are never stretched (which blurs them): the largest cover in a grid,
// on the book page and on the player is about 280 points, times the display scaling
static SIZE: AtomicU32 = AtomicU32::new(420);
const BUDGET: usize = 32 * 1024 * 1024; // decoded covers kept in memory, in bytes
const WORKERS: usize = 3;

/// Sets the cover size for a display scaling factor (1.5 at 150%). Call before covers are asked for.
pub fn set_scale(scale: f32) {
    SIZE.store(((280.0 * scale.clamp(1.0, 3.0)) as u32).div_ceil(20) * 20, Relaxed);
}

/// How many covers fit in the memory budget at the current size.
fn keep() -> usize {
    let s = SIZE.load(Relaxed) as usize;
    (BUDGET / (s * s * 4)).max(24)
}

const MAX_AGE: std::time::Duration = std::time::Duration::from_secs(30 * 24 * 3600); // then fetched again, in case the cover changed

/// Where a book's cover is kept on disk, already squared to the screen size, so it isn't downloaded on every launch.
fn file(id: &str) -> Option<std::path::PathBuf> {
    let safe: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect(); // ids come from the server
    Some(dirs::cache_dir()?.join("lectern-native").join("covers").join(safe + ".jpg"))
}

/// The cover from disk when it is there, fresh and the right size; otherwise from the server, saved for next time.
/// Offline, an old saved copy is still better than none.
fn load(client: &Client, id: &str, size: u32) -> Option<image::RgbaImage> {
    let path = file(id);
    let saved = path.as_ref().and_then(|p| image::open(p).ok()).map(|i| i.to_rgba8()).filter(|i| i.dimensions() == (size, size));
    let fresh = path.as_ref().and_then(|p| p.metadata().ok()?.modified().ok()?.elapsed().ok()).is_some_and(|age| age < MAX_AGE);
    if fresh && saved.is_some() {
        return saved;
    }
    match client.cover(id, size) {
        Ok(img) => {
            if let Some(p) = &path {
                let _ = save(p, &img);
            }
            Some(img)
        }
        Err(_) => saved,
    }
}

fn save(path: &std::path::Path, img: &image::RgbaImage) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(path.parent().unwrap())?;
    let rgb = image::DynamicImage::ImageRgba8(img.clone()).to_rgb8(); // jpeg has no alpha, and covers have none to lose
    image::codecs::jpeg::JpegEncoder::new_with_quality(std::fs::File::create(path)?, 92).encode_image(&rgb)?;
    Ok(())
}

type Job = (Client, String);
type Done = Box<dyn Fn(String, Option<slint::SharedPixelBuffer<slint::Rgba8Pixel>>) + Send + Sync>;

pub struct Covers {
    cache: HashMap<String, slint::Image>,
    order: VecDeque<String>, // least recently used first
    pending: HashSet<String>,
    queue: Arc<(Mutex<Vec<Job>>, Condvar)>,
}

impl Covers {
    /// `done` runs on a worker thread with each finished cover (None when the book has none).
    pub fn new(done: Done) -> Self {
        let queue: Arc<(Mutex<Vec<Job>>, Condvar)> = Arc::default();
        let done = Arc::new(done);
        for _ in 0..WORKERS {
            let (q, done) = (queue.clone(), done.clone());
            std::thread::spawn(move || loop {
                let (client, id) = {
                    let (lock, cv) = &*q;
                    let mut jobs = cv.wait_while(lock.lock().unwrap(), |j| j.is_empty()).unwrap();
                    jobs.pop().unwrap()
                };
                let px = load(&client, &id, SIZE.load(Relaxed)).map(|img| {
                    let (w, h) = img.dimensions();
                    slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(img.as_raw(), w, h)
                });
                done(id, px);
            });
        }
        Covers { cache: HashMap::new(), order: VecDeque::new(), pending: HashSet::new(), queue }
    }

    /// The cover if it is cached (and marks it as recently used).
    pub fn get(&mut self, id: &str) -> Option<slint::Image> {
        let img = self.cache.get(id)?.clone();
        self.touch(id);
        Some(img)
    }

    /// Asks for a cover unless it is cached or already on its way.
    pub fn want(&mut self, client: &Client, id: &str) {
        if self.cache.contains_key(id) {
            self.touch(id);
            return;
        }
        if self.pending.insert(id.to_string()) {
            let (lock, cv) = &*self.queue;
            lock.lock().unwrap().push((client.clone(), id.to_string()));
            cv.notify_one();
        }
    }

    /// Forgets queued requests, e.g. when the library list changes; covers already on their way still arrive.
    pub fn drop_queued(&mut self) {
        let mut jobs = self.queue.0.lock().unwrap();
        for (_, id) in jobs.drain(..) {
            self.pending.remove(&id);
        }
    }

    /// Stores a finished cover. Returns the ids pushed out of the cache, whose rows should drop their image.
    pub fn insert(&mut self, id: String, img: slint::Image) -> Vec<String> {
        self.pending.remove(&id);
        self.cache.insert(id.clone(), img);
        self.touch(&id);
        let mut evicted = vec![];
        while self.order.len() > keep() {
            if let Some(old) = self.order.pop_front() {
                self.cache.remove(&old);
                evicted.push(old);
            }
        }
        evicted
    }

    pub fn failed(&mut self, id: &str) {
        self.pending.remove(id);
    }

    fn touch(&mut self, id: &str) {
        if let Some(i) = self.order.iter().position(|x| x == id) {
            self.order.remove(i);
        }
        self.order.push_back(id.to_string());
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn saved_covers_come_back_and_stay_in_their_folder() {
        assert!(super::file("../../evil").unwrap().ends_with("lectern-native/covers/evil.jpg"));
        let p = std::env::temp_dir().join("lectern-cover-test.jpg");
        super::save(&p, &image::RgbaImage::from_pixel(60, 60, image::Rgba([200, 100, 50, 255]))).unwrap();
        assert_eq!(image::open(&p).unwrap().to_rgba8().dimensions(), (60, 60));
        let _ = std::fs::remove_file(p);
    }
}
