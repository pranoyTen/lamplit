// Offline downloads: a queue that saves each book's audio files to disk, one book at a time, on a worker thread.
// Files come down in Range chunks and the byte count is saved as it grows, so a paused, cancelled or crashed download resumes where it
// stopped. The list lives in a JSON file next to the app's settings. Same behaviour as the Tauri app's downloads.js.
use crate::api::{Client, Track};
use crate::book::Chapter;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Status {
    Queued,
    Active,
    Done,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct File {
    pub path: String, // on the server
    pub name: String, // on disk, inside the book's folder
    pub size: u64,    // 0 while unknown
    pub start: f64,
    pub got: u64,
    pub done: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub author: String,
    pub status: Status,
    pub error: String,
    pub total: u64,
    pub folder: PathBuf, // the downloads folder chosen when this book started; the book's files go in folder/id
    pub duration: f64,
    pub chapters: Vec<Chapter>,
    pub files: Vec<File>,
}

impl Item {
    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|f| f.got).sum()
    }
    pub fn dir(&self) -> PathBuf {
        self.folder.join(&self.id)
    }
    pub fn fraction(&self) -> f32 {
        if self.total == 0 { 0.0 } else { (self.bytes() as f64 / self.total as f64).min(1.0) as f32 }
    }
}

struct St {
    items: Vec<Item>,
    paused: bool,
    client: Option<Client>,
    folder: PathBuf,
    limit: u64, // bytes, 0 = no limit
    manifest: PathBuf,
    chunk: u64,
    last_note: Instant,
}

struct Shared {
    st: Mutex<St>,
    cv: Condvar,
    changed: Box<dyn Fn() + Send + Sync>,
    allowed: Box<dyn Fn() -> bool + Send + Sync>, // false holds the queue, e.g. on a metered connection; asked again every 30 seconds
}

#[derive(Clone)]
pub struct Downloads(Arc<Shared>);

enum Fail {
    Stopped, // paused or removed: not an error
    Msg(String),
}

impl From<String> for Fail {
    fn from(s: String) -> Self {
        Fail::Msg(s)
    }
}

fn disk(e: std::io::Error) -> Fail {
    Fail::Msg(format!("Couldn't write the file: {e}"))
}

impl Downloads {
    /// Loads the saved list and starts the worker. `changed` runs on the worker thread whenever the list or a download's progress
    /// changes (progress at most a few times a second).
    pub fn new(manifest: PathBuf, folder: PathBuf, limit: u64, chunk: u64, changed: impl Fn() + Send + Sync + 'static, allowed: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        let mut items: Vec<Item> = fs::read_to_string(&manifest).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        for it in &mut items {
            if it.status == Status::Active {
                it.status = Status::Queued; // the app closed mid-download
            }
        }
        let sh = Arc::new(Shared {
            st: Mutex::new(St { items, paused: false, client: None, folder, limit, manifest, chunk, last_note: Instant::now() }),
            cv: Condvar::new(),
            changed: Box::new(changed),
            allowed: Box::new(allowed),
        });
        let worker = sh.clone();
        std::thread::spawn(move || work(&worker));
        Downloads(sh)
    }

    fn edit<R>(&self, f: impl FnOnce(&mut St) -> R) -> R {
        let r = f(&mut self.0.st.lock().unwrap());
        save(&self.0.st.lock().unwrap());
        self.0.cv.notify_all();
        (self.0.changed)();
        r
    }

    /// Downloads wait until there is a server to download from.
    pub fn set_client(&self, c: Option<Client>) {
        self.edit(|s| s.client = c);
    }
    pub fn set_folder(&self, f: PathBuf) {
        self.edit(|s| s.folder = f);
    }
    pub fn set_limit(&self, bytes: u64) {
        self.edit(|s| s.limit = bytes);
    }
    pub fn folder(&self) -> PathBuf {
        self.0.st.lock().unwrap().folder.clone()
    }
    pub fn limit(&self) -> u64 {
        self.0.st.lock().unwrap().limit
    }
    pub fn paused(&self) -> bool {
        self.0.st.lock().unwrap().paused
    }

    pub fn add(&self, id: &str, title: &str, author: &str) {
        self.edit(|s| {
            if !s.items.iter().any(|i| i.id == id) {
                s.items.push(Item {
                    id: id.into(), title: title.into(), author: author.into(), status: Status::Queued, error: String::new(), total: 0,
                    folder: s.folder.clone(), duration: 0.0, chapters: vec![], files: vec![],
                });
            }
        });
    }

    /// Forgets a book and deletes its files (the worker deletes them if the book is downloading right now).
    pub fn remove(&self, id: &str) {
        let gone = self.edit(|s| {
            let i = s.items.iter().position(|x| x.id == id)?;
            let it = s.items.remove(i);
            (it.status != Status::Active).then(|| it.dir())
        });
        if let Some(dir) = gone {
            std::thread::spawn(move || fs::remove_dir_all(dir));
        }
    }

    pub fn remove_finished(&self) {
        for id in self.list().into_iter().filter(|i| i.status == Status::Done).map(|i| i.id) {
            self.remove(&id);
        }
    }

    pub fn retry(&self, id: &str) {
        self.edit(|s| {
            if let Some(it) = s.items.iter_mut().find(|i| i.id == id && i.status == Status::Error) {
                it.status = Status::Queued;
                it.error.clear();
            }
        });
    }

    pub fn pause(&self, on: bool) {
        self.edit(|s| s.paused = on);
    }

    pub fn list(&self) -> Vec<Item> {
        self.0.st.lock().unwrap().items.clone()
    }

    pub fn get(&self, id: &str) -> Option<Item> {
        self.0.st.lock().unwrap().items.iter().find(|i| i.id == id).cloned()
    }

    /// Bytes on disk across every book.
    pub fn bytes(&self) -> u64 {
        self.0.st.lock().unwrap().items.iter().map(Item::bytes).sum()
    }

    /// A finished book's files to play, or None when it isn't downloaded or a file is missing or cut short (then it streams instead).
    pub fn local(&self, id: &str) -> Option<(Vec<Track>, f64, Vec<Chapter>)> {
        let it = self.get(id).filter(|i| i.status == Status::Done)?;
        let dir = it.dir();
        let mut tracks = vec![];
        for f in &it.files {
            let p = dir.join(&f.name);
            if fs::metadata(&p).map(|m| m.len()).ok()? != f.size {
                return None;
            }
            tracks.push(Track { url: p.to_string_lossy().into(), start: f.start });
        }
        Some((tracks, it.duration, it.chapters))
    }
}

fn save(s: &St) {
    if let Some(dir) = s.manifest.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let tmp = s.manifest.with_extension("tmp");
    if fs::write(&tmp, serde_json::to_string(&s.items).unwrap_or_default()).is_ok() {
        let _ = fs::rename(&tmp, &s.manifest);
    }
}

fn work(sh: &Shared) {
    loop {
        let (id, client) = {
            let ready = |s: &mut St| !s.paused && s.client.is_some() && s.items.iter().any(|i| i.status == Status::Queued);
            let mut s = sh.st.lock().unwrap();
            // wait for work; when it is held back (a metered connection), look again every 30 seconds
            while !ready(&mut s) || !(sh.allowed)() {
                s = sh.cv.wait_timeout(s, Duration::from_secs(30)).unwrap().0;
            }
            let it = s.items.iter_mut().find(|i| i.status == Status::Queued).unwrap();
            it.status = Status::Active;
            it.error.clear();
            let id = it.id.clone();
            save(&s);
            (id, s.client.clone().unwrap())
        };
        (sh.changed)();
        let r = download(sh, &id, &client);
        {
            let mut s = sh.st.lock().unwrap();
            match s.items.iter_mut().find(|i| i.id == id) {
                Some(it) => match r {
                    Ok(()) => it.status = Status::Done,
                    Err(Fail::Stopped) => it.status = Status::Queued, // paused: it waits in the queue
                    Err(Fail::Msg(e)) => (it.status, it.error) = (Status::Error, e),
                },
                // removed while downloading: its files are deleted now that nothing is writing to them
                None => {
                    let dir = s.folder.join(&id);
                    let _ = fs::remove_dir_all(dir);
                }
            }
            save(&s);
        }
        (sh.changed)();
    }
}

// keeps going while the book is still on the list and downloads aren't paused
fn carry_on(sh: &Shared, id: &str) -> Result<(), Fail> {
    let s = sh.st.lock().unwrap();
    if s.paused || !s.items.iter().any(|i| i.id == id) { Err(Fail::Stopped) } else { Ok(()) }
}

// records progress on file `n`; saves the list and tells the UI now and then
fn progress(sh: &Shared, id: &str, n: usize, got: u64, size: Option<u64>, force: bool) {
    let note = {
        let mut s = sh.st.lock().unwrap();
        if let Some(f) = s.items.iter_mut().find(|i| i.id == id).and_then(|i| i.files.get_mut(n)) {
            f.got = got;
            if let Some(sz) = size {
                f.size = sz;
            }
        }
        let due = force || s.last_note.elapsed() > Duration::from_millis(400);
        if due {
            s.last_note = Instant::now();
            save(&s);
        }
        due
    };
    if note {
        (sh.changed)();
    }
}

fn download(sh: &Shared, id: &str, client: &Client) -> Result<(), Fail> {
    // the first run fetches the file list and checks the storage limit
    if sh.st.lock().unwrap().items.iter().find(|i| i.id == id).is_some_and(|i| i.files.is_empty()) {
        let (files, duration, chapters) = client.files(id)?;
        let total: u64 = files.iter().map(|f| f.size).sum();
        let mut s = sh.st.lock().unwrap();
        let used: u64 = s.items.iter().filter(|i| i.id != id).map(Item::bytes).sum();
        if s.limit > 0 && used + total > s.limit {
            return Err(Fail::Msg("Over the storage limit. Raise it in Settings or remove books".into()));
        }
        let folder = s.folder.clone();
        let Some(it) = s.items.iter_mut().find(|i| i.id == id) else { return Err(Fail::Stopped) };
        it.folder = folder;
        it.total = total;
        it.duration = duration;
        it.chapters = chapters;
        it.files = files.iter().enumerate()
            .map(|(n, f)| File { path: f.path.clone(), name: format!("{n}{}", f.ext), size: f.size, start: f.start, got: 0, done: false })
            .collect();
        save(&s);
    }
    let (dir, files, chunk) = {
        let s = sh.st.lock().unwrap();
        let it = s.items.iter().find(|i| i.id == id).ok_or(Fail::Stopped)?;
        (it.dir(), it.files.clone(), s.chunk)
    };
    fs::create_dir_all(&dir).map_err(disk)?;
    for (n, f) in files.iter().enumerate().filter(|(_, f)| !f.done) {
        let (got, size) = fetch_file(sh, id, client, n, f, &dir.join(&f.name), chunk)?;
        let mut s = sh.st.lock().unwrap();
        if let Some(x) = s.items.iter_mut().find(|i| i.id == id).and_then(|i| i.files.get_mut(n)) {
            (x.got, x.size, x.done) = (got, size, true);
        }
        save(&s);
    }
    Ok(())
}

/// Downloads one file from where it stopped; returns its final byte count and size.
fn fetch_file(sh: &Shared, id: &str, client: &Client, n: usize, f: &File, path: &Path, chunk: u64) -> Result<(u64, u64), Fail> {
    // trust the file, not the counter: writing past its real end would leave a hole that no player can decode
    let mut got = f.got.min(fs::metadata(path).map(|m| m.len()).unwrap_or(0));
    let mut size = f.size;
    let mut out = OpenOptions::new().create(true).truncate(false).write(true).open(path).map_err(disk)?;
    out.set_len(got).map_err(disk)?;
    out.seek(SeekFrom::Start(got)).map_err(disk)?;
    let mut buf = vec![0u8; 64 * 1024];
    while size == 0 || got < size {
        carry_on(sh, id)?;
        let to = if size > 0 { (got + chunk).min(size) - 1 } else { got + chunk - 1 };
        let mut res = client.range(&f.path, got, to)?;
        // a server that ignores Range sends the whole file: start it over from the top
        let whole = res.status().as_u16() == 200;
        if whole && got > 0 {
            got = 0;
            out.set_len(0).map_err(disk)?;
            out.seek(SeekFrom::Start(0)).map_err(disk)?;
        }
        let before = got;
        loop {
            let k = res.read(&mut buf).map_err(|_| Fail::Msg("Can't reach the server. Check your connection".into()))?;
            if k == 0 {
                break;
            }
            out.write_all(&buf[..k]).map_err(disk)?;
            got += k as u64;
            progress(sh, id, n, got, None, false);
            carry_on(sh, id)?;
        }
        out.flush().map_err(disk)?;
        progress(sh, id, n, got, None, true);
        if got == before {
            return Err(Fail::Msg("The server sent no data".into()));
        }
        if whole || (size == 0 && got - before < chunk) {
            size = got; // the end of a file whose size wasn't known
        }
    }
    progress(sh, id, n, got, Some(size), true);
    Ok((got, size))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;
    use std::net::TcpListener;

    // a tiny server: one book with one 20 000-byte audio file, served in Range pieces like Audiobookshelf
    fn server(body: Vec<u8>) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("http://{}", l.local_addr().unwrap());
        std::thread::spawn(move || {
            for c in l.incoming().flatten() {
                let body = body.clone();
                std::thread::spawn(move || {
                    let mut r = std::io::BufReader::new(c.try_clone().unwrap());
                    let (mut line, mut range) = (String::new(), None);
                    r.read_line(&mut line).unwrap();
                    loop {
                        let mut h = String::new();
                        r.read_line(&mut h).unwrap();
                        if h.trim().is_empty() {
                            break;
                        }
                        if let Some(v) = h.to_ascii_lowercase().strip_prefix("range: bytes=") {
                            let (a, b) = v.trim().split_once('-').unwrap();
                            range = Some((a.parse::<usize>().unwrap(), b.parse::<usize>().unwrap()));
                        }
                    }
                    let mut c = c;
                    if line.contains("/file/") {
                        let (a, b) = range.unwrap();
                        let b = b.min(body.len() - 1);
                        let part = &body[a..=b];
                        let _ = write!(c, "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {a}-{b}/{}\r\nConnection: close\r\n\r\n", part.len(), body.len());
                        let _ = c.write_all(part);
                    } else {
                        let json = format!(r#"{{"media":{{"duration":60,"chapters":[],"audioFiles":[{{"index":1,"ino":"7","duration":60,"metadata":{{"ext":".mp3","size":{}}}}}]}}}}"#, body.len());
                        let _ = write!(c, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}", json.len());
                    }
                });
            }
        });
        addr
    }

    fn wait_for(d: &Downloads, id: &str, status: Status) -> Item {
        for _ in 0..500 {
            if let Some(i) = d.get(id).filter(|i| i.status == status) {
                return i;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("never reached {status:?}: {:?}", d.get(id));
    }

    #[test]
    fn downloads_in_pieces_and_resumes() {
        let body: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
        let addr = server(body.clone());
        let tmp = std::env::temp_dir().join(format!("lectern-dl-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let client = Client::new(&addr, "t").unwrap();

        // pretend an earlier run stopped 5 000 bytes in, with only 3 000 of them really on disk
        let d = Downloads::new(tmp.join("downloads.json"), tmp.join("books"), 0, 4096, || {}, || true);
        d.add("b1", "Book", "Author");
        {
            let mut s = d.0.st.lock().unwrap();
            let it = &mut s.items[0];
            it.total = 20_000;
            it.files = vec![File { path: "/api/items/b1/file/7".into(), name: "0.mp3".into(), size: 20_000, start: 0.0, got: 5_000, done: false }];
        }
        fs::create_dir_all(tmp.join("books/b1")).unwrap();
        fs::write(tmp.join("books/b1/0.mp3"), &body[..3_000]).unwrap();
        d.set_client(Some(client.clone()));
        wait_for(&d, "b1", Status::Done);
        assert_eq!(fs::read(tmp.join("books/b1/0.mp3")).unwrap(), body, "resumed from what was really on disk");
        let (tracks, ..) = d.local("b1").expect("a complete copy plays locally");
        assert_eq!(tracks.len(), 1);

        // a fresh book fetches its file list, then the file in 4 KB pieces
        d.add("b2", "Book 2", "Author");
        let it = wait_for(&d, "b2", Status::Done);
        assert_eq!(it.total, 20_000);
        assert_eq!(fs::read(tmp.join("books/b2/0.mp3")).unwrap(), body);

        // the list survives a restart
        let again = Downloads::new(tmp.join("downloads.json"), tmp.join("books"), 0, 4096, || {}, || true);
        assert_eq!(again.list().len(), 2);

        // over the limit: refused with a message, nothing written
        d.set_limit(30_000);
        d.add("b3", "Book 3", "Author");
        let it = wait_for(&d, "b3", Status::Error);
        assert!(it.error.contains("storage limit"), "{}", it.error);

        // removing deletes the files
        d.remove("b1");
        std::thread::sleep(Duration::from_millis(200));
        assert!(!tmp.join("books/b1").exists());
        let _ = fs::remove_dir_all(&tmp);
    }
}
