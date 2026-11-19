// Audiobookshelf REST client, the native counterpart of src/api.js. Blocking: callers run it off the UI thread.
use crate::book::{Bookmark, Chapter};
use serde_json::{json, Value};

#[derive(Clone)]
pub struct Client {
    pub base: String,
    pub token: String,
    http: reqwest::blocking::Client,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Book {
    pub id: String,
    pub title: String,
    pub author: String,
    pub duration: f64,
    pub added: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Series {
    pub id: String,
    pub name: String,
    pub books: Vec<Book>,
}

#[derive(Clone, Debug, Default)]
pub struct Listened {
    pub id: String,
    pub title: String,
    pub author: String,
    pub secs: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Listening {
    pub days: std::collections::HashMap<String, f64>,
    pub items: Vec<Listened>,
    pub finished: Vec<(String, f64)>, // book id and when, in milliseconds since 1970
}

/// Everything the book screen shows.
#[derive(Clone, Debug, Default)]
pub struct Details {
    pub title: String,
    pub subtitle: String,
    pub authors: String,
    pub narrators: String,
    pub series: String,
    pub genres: String,
    pub published: String,
    pub language: String,
    pub description: String,
    pub duration: f64,
    pub chapters: usize,
    pub progress: f64,
    pub finished: bool,
}

fn book_of(i: &Value) -> Book {
    let md = &i["media"]["metadata"];
    Book {
        id: s(&i["id"]),
        title: s(&md["title"]),
        author: md["authorName"].as_str().or(md["author"].as_str()).unwrap_or_default().into(),
        duration: i["media"]["duration"].as_f64().unwrap_or(0.0),
        added: i["addedAt"].as_f64().unwrap_or(0.0),
    }
}

/// Book descriptions are HTML: keep the paragraphs, drop the tags.
pub fn plain_text(html: &str) -> String {
    let mut out = String::new();
    let mut tag = String::new();
    let mut in_tag = false;
    for ch in html.chars() {
        match (in_tag, ch) {
            (false, '<') => (in_tag, tag) = (true, String::new()),
            (true, '>') => {
                in_tag = false;
                let name = tag.trim_start_matches('/').split_whitespace().next().unwrap_or("").to_ascii_lowercase();
                if matches!(name.as_str(), "p" | "br" | "div" | "br/") && !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            (true, c) => tag.push(c),
            (false, c) => out.push(c),
        }
    }
    let out = out.replace("&amp;", "&").replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&nbsp;", " ");
    out.trim().to_string()
}

/// One audio file to play: `url` is a web address to stream, or a path to a downloaded file.
#[derive(Clone, Debug)]
pub struct Track {
    pub url: String,
    pub start: f64,
}

/// One audio file on the server, as the download manager needs it.
#[derive(Clone, Debug)]
pub struct RemoteFile {
    pub path: String, // the server path, without the address
    pub ext: String,
    pub size: u64,
    pub start: f64,
}

fn chapters_of(v: &Value, duration: f64) -> Vec<Chapter> {
    let list = v.as_array().cloned().unwrap_or_default();
    list.iter().enumerate()
        .map(|(i, c)| {
            let start = c["start"].as_f64().unwrap_or(0.0);
            let end = c["end"].as_f64().or_else(|| list.get(i + 1).and_then(|n| n["start"].as_f64())).unwrap_or(duration);
            Chapter { start, end: end.max(start + 1.0), title: s(&c["title"]) }
        })
        .collect()
}

#[derive(Clone, Debug)]
pub struct Session {
    pub id: String,
    pub start: f64,
    pub duration: f64,
    pub tracks: Vec<Track>,
    pub chapters: Vec<Chapter>,
}

fn base_of(server: &str) -> Result<String, String> {
    let b = server.trim().trim_end_matches('/').to_string();
    if b.starts_with("http://") || b.starts_with("https://") { Ok(b) } else { Err("Start the address with http:// or https://".into()) }
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

impl Client {
    pub fn new(server: &str, token: &str) -> Result<Self, String> {
        Ok(Client { base: base_of(server)?, token: token.into(), http: reqwest::blocking::Client::new() })
    }

    fn call(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value, String> {
        let mut r = self.http.request(method, format!("{}{path}", self.base));
        if !self.token.is_empty() {
            r = r.bearer_auth(&self.token);
        }
        if let Some(b) = body {
            r = r.json(&b);
        }
        let res = r.send().map_err(|e| format!("Can't reach the server: {e}"))?;
        let status = res.status();
        if !status.is_success() {
            return Err(if status.as_u16() == 401 { "Wrong username or password, or the session expired".into() } else { format!("The server answered {status}") });
        }
        Ok(res.json().unwrap_or(Value::Null))
    }

    /// Signs in and returns a client holding the user's token.
    pub fn login(server: &str, username: &str, password: &str) -> Result<Self, String> {
        let c = Client::new(server, "")?;
        let v = c.call(reqwest::Method::POST, "/login", Some(json!({ "username": username, "password": password })))?;
        let u = &v["user"];
        let token = u["token"].as_str().or(u["accessToken"].as_str()).ok_or("The server sent no token")?;
        Client::new(&c.base, token)
    }

    /// The first page of books in every book library.
    // shortcut: first 100 books per library, no paging; add paging with the real library screen
    pub fn books(&self) -> Result<Vec<Book>, String> {
        let mut out = vec![];
        for lib in self.book_libraries()? {
            let r = self.call(reqwest::Method::GET, &format!("/api/libraries/{lib}/items?limit=100&sort=media.metadata.title"), None)?;
            out.extend(r["results"].as_array().into_iter().flatten().map(book_of));
        }
        Ok(out)
    }

    fn book_libraries(&self) -> Result<Vec<String>, String> {
        let libs = self.call(reqwest::Method::GET, "/api/libraries", None)?;
        Ok(libs["libraries"].as_array().into_iter().flatten().filter(|l| l["mediaType"] != "podcast").map(|l| s(&l["id"])).collect())
    }

    /// Every series in the book libraries, each with its books in reading order.
    pub fn series(&self) -> Result<Vec<Series>, String> {
        let mut out = vec![];
        for lib in self.book_libraries()? {
            let r = self.call(reqwest::Method::GET, &format!("/api/libraries/{lib}/series?limit=200&sort=name"), None)?;
            out.extend(r["results"].as_array().into_iter().flatten().map(|x| Series {
                id: s(&x["id"]),
                name: s(&x["name"]),
                books: x["books"].as_array().into_iter().flatten().map(book_of).collect(),
            }));
        }
        Ok(out)
    }

    /// Listening time per day and per book, and when books were finished.
    pub fn listening(&self) -> Result<Listening, String> {
        let st = self.call(reqwest::Method::GET, "/api/me/listening-stats", None)?;
        let me = self.call(reqwest::Method::GET, "/api/me", None)?;
        let days = st["days"].as_object().into_iter().flatten().map(|(k, v)| (k.clone(), v.as_f64().unwrap_or(0.0))).collect();
        let items = st["items"].as_object().into_iter().flatten()
            .map(|(_, i)| {
                let md = &i["mediaMetadata"];
                let author = md["authorName"].as_str().map(str::to_string)
                    .or_else(|| md["authors"][0]["name"].as_str().map(str::to_string))
                    .or_else(|| md["author"].as_str().map(str::to_string))
                    .unwrap_or_default();
                Listened { id: s(&i["id"]), title: s(&md["title"]), author, secs: i["timeListening"].as_f64().or(i["timeListened"].as_f64()).unwrap_or(0.0) }
            })
            .collect();
        let finished = me["mediaProgress"].as_array().into_iter().flatten()
            .filter(|p| p["isFinished"].as_bool() == Some(true))
            .map(|p| (s(&p["libraryItemId"]), p["finishedAt"].as_f64().or(p["lastUpdate"].as_f64()).unwrap_or(0.0)))
            .collect();
        Ok(Listening { days, items, finished })
    }

    /// How far the user is in each book: 0..1, and 1 for finished.
    pub fn progress(&self) -> Result<std::collections::HashMap<String, f64>, String> {
        let me = self.call(reqwest::Method::GET, "/api/me", None)?;
        Ok(me["mediaProgress"].as_array().into_iter().flatten()
            .map(|p| (s(&p["libraryItemId"]), if p["isFinished"].as_bool() == Some(true) { 1.0 } else { p["progress"].as_f64().unwrap_or(0.0) }))
            .collect())
    }

    /// Books started and not finished, most recent first.
    pub fn in_progress(&self) -> Result<Vec<Book>, String> {
        let r = self.call(reqwest::Method::GET, "/api/me/items-in-progress", None)?;
        Ok(r["libraryItems"].as_array().into_iter().flatten().filter(|i| i["mediaType"] != "podcast").map(book_of).collect())
    }

    pub fn details(&self, id: &str) -> Result<Details, String> {
        let i = self.call(reqwest::Method::GET, &format!("/api/items/{id}?expanded=1&include=progress"), None)?;
        let (m, md, p) = (&i["media"], &i["media"]["metadata"], &i["userMediaProgress"]);
        let names = |v: &Value, key: &str| v.as_array().into_iter().flatten().map(|x| if key.is_empty() { s(x) } else { s(&x[key]) }).filter(|x| !x.is_empty()).collect::<Vec<_>>().join(", ");
        let series = md["series"].as_array().into_iter().flatten()
            .map(|x| match x["sequence"].as_str() { Some(n) if !n.is_empty() => format!("{} #{n}", s(&x["name"])), _ => s(&x["name"]) })
            .collect::<Vec<_>>().join(", ");
        let year = md["publishedYear"].as_str().map(str::to_string).or(md["publishedYear"].as_i64().map(|y| y.to_string())).unwrap_or_default();
        Ok(Details {
            title: s(&md["title"]),
            subtitle: s(&md["subtitle"]),
            authors: names(&md["authors"], "name"),
            narrators: names(&md["narrators"], ""),
            series,
            genres: names(&md["genres"], ""),
            published: [s(&md["publisher"]), year].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(", "),
            language: s(&md["language"]),
            description: plain_text(md["description"].as_str().unwrap_or_default()),
            duration: m["duration"].as_f64().unwrap_or(0.0),
            chapters: m["chapters"].as_array().map_or(0, Vec::len),
            progress: p["progress"].as_f64().unwrap_or(0.0),
            finished: p["isFinished"].as_bool().unwrap_or(false),
        })
    }

    pub fn set_finished(&self, id: &str, finished: bool) -> Result<(), String> {
        self.call(reqwest::Method::PATCH, &format!("/api/me/progress/{id}"), Some(json!({ "isFinished": finished }))).map(|_| ())
    }

    /// A cover, decoded and scaled to fit `size` × `size`, as RGBA pixels.
    pub fn cover(&self, id: &str, size: u32) -> Result<image::RgbaImage, String> {
        let res = self.http.get(format!("{}/api/items/{id}/cover?width={size}", self.base)).bearer_auth(&self.token).send().map_err(|e| e.to_string())?;
        if !res.status().is_success() {
            return Err(format!("no cover ({})", res.status()));
        }
        let bytes = res.bytes().map_err(|e| e.to_string())?;
        let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
        Ok(img.thumbnail(size, size).to_rgba8())
    }

    /// Opens a playback session: where the server last left off, and the audio files in play order.
    pub fn play(&self, id: &str) -> Result<Session, String> {
        let body = json!({ "deviceInfo": { "clientName": "Lectern" }, "forceDirectPlay": true,
            "supportedMimeTypes": ["audio/mpeg", "audio/mp4", "audio/flac", "audio/ogg"] });
        let v = self.call(reqwest::Method::POST, &format!("/api/items/{id}/play"), Some(body))?;
        let tracks = v["audioTracks"].as_array().into_iter().flatten()
            .map(|t| Track { url: format!("{}{}", self.base, s(&t["contentUrl"])), start: t["startOffset"].as_f64().unwrap_or(0.0) })
            .collect::<Vec<_>>();
        if tracks.is_empty() {
            return Err("This book has no audio".into());
        }
        let duration = v["duration"].as_f64().unwrap_or(0.0);
        Ok(Session { id: s(&v["id"]), start: v["currentTime"].as_f64().unwrap_or(0.0), duration, tracks, chapters: chapters_of(&v["chapters"], duration) })
    }

    /// The audio files of a book to download, in play order, with its length and chapters.
    pub fn files(&self, id: &str) -> Result<(Vec<RemoteFile>, f64, Vec<Chapter>), String> {
        let m = self.call(reqwest::Method::GET, &format!("/api/items/{id}?expanded=1"), None)?["media"].take();
        let mut list: Vec<&Value> = m["audioFiles"].as_array().into_iter().flatten().collect();
        list.sort_by_key(|f| f["index"].as_i64().unwrap_or(0));
        let mut start = 0.0;
        let files: Vec<RemoteFile> = list.iter()
            .map(|f| {
                let md = &f["metadata"];
                let ext = md["ext"].as_str().map(str::to_string)
                    .unwrap_or_else(|| s(&md["filename"]).rsplit_once('.').map(|(_, e)| format!(".{e}")).unwrap_or_default());
                let duration = f["duration"].as_f64().unwrap_or(0.0);
                let r = RemoteFile { path: format!("/api/items/{id}/file/{}", s(&f["ino"])), ext, size: md["size"].as_u64().unwrap_or(0), start };
                start += duration;
                r
            })
            .collect();
        if files.is_empty() {
            return Err("This book has no audio".into());
        }
        let duration = m["duration"].as_f64().unwrap_or(start);
        Ok((files, duration, chapters_of(&m["chapters"], duration)))
    }

    /// Fetches `bytes=from-to` of a server file (`to` inclusive), with this user's token.
    pub fn range(&self, path: &str, from: u64, to: u64) -> Result<reqwest::blocking::Response, String> {
        let res = self.http.get(format!("{}{path}", self.base)).bearer_auth(&self.token).header("Range", format!("bytes={from}-{to}")).send()
            .map_err(|_| "Can't reach the server. Check your connection".to_string())?;
        if !res.status().is_success() {
            return Err(format!("The server answered {}", res.status()));
        }
        Ok(res)
    }

    /// This user's bookmarks in one book, in book order.
    pub fn bookmarks(&self, id: &str) -> Result<Vec<Bookmark>, String> {
        let me = self.call(reqwest::Method::GET, "/api/me", None)?;
        let mut b: Vec<Bookmark> = me["bookmarks"].as_array().into_iter().flatten()
            .filter(|b| b["libraryItemId"] == id)
            .map(|b| Bookmark { time: b["time"].as_f64().unwrap_or(0.0), title: s(&b["title"]) })
            .collect();
        b.sort_by(|x, y| x.time.total_cmp(&y.time));
        Ok(b)
    }

    pub fn add_bookmark(&self, id: &str, b: &Bookmark) -> Result<(), String> {
        self.call(reqwest::Method::POST, &format!("/api/me/item/{id}/bookmark"), Some(json!({ "time": b.time, "title": b.title }))).map(|_| ())
    }

    pub fn remove_bookmark(&self, id: &str, time: f64) -> Result<(), String> {
        self.call(reqwest::Method::DELETE, &format!("/api/me/item/{id}/bookmark/{time}"), None).map(|_| ())
    }

    /// Reports the place and the seconds listened since the last report; `close` ends the session.
    pub fn sync(&self, session: &str, current: f64, listened: f64, close: bool) -> Result<(), String> {
        let path = format!("/api/session/{session}/{}", if close { "close" } else { "sync" });
        self.call(reqwest::Method::POST, &path, Some(json!({ "currentTime": current, "timeListened": listened }))).map(|_| ())
    }

    /// The cover as a plain URL (the token rides in the query), for places that fetch it themselves, like the OS media overlay.
    pub fn cover_url(&self, id: &str) -> String {
        format!("{}/api/items/{id}/cover?width=400&token={}", self.base, self.token)
    }

    /// A blocking HTTP client that sends this user's token, for streaming audio.
    pub fn http(&self) -> (reqwest::blocking::Client, String) {
        (self.http.clone(), self.token.clone())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn descriptions_become_plain_text() {
        assert_eq!(super::plain_text("<p>One &amp; two.</p><p>Three<br/>four</p>"), "One & two.\nThree\nfour");
        assert_eq!(super::plain_text("No tags"), "No tags");
    }
}
