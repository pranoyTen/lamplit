// Cover images: fetched and decoded by a few worker threads, scaled to thumbnails, and kept in a small cache so memory stays bounded.
// Requests are served newest first, so after a fast scroll the covers now on screen come before the ones scrolled past.
use crate::api::Client;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Condvar, Mutex};

pub const SIZE: u32 = 240; // pixels: sharp in a 168 px grid cell at 125-150% display scaling
const KEEP: usize = 150; // about 35 MB of decoded covers at most
const WORKERS: usize = 3;

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
                let px = client.cover(&id, SIZE).ok().map(|img| {
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
        while self.order.len() > KEEP {
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
