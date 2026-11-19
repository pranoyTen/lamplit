// Audiobookshelf REST client, the native counterpart of src/api.js. Blocking: callers run it off the UI thread.
use serde_json::{json, Value};

#[derive(Clone)]
pub struct Client {
    pub base: String,
    pub token: String,
    http: reqwest::blocking::Client,
}

#[derive(Clone, Debug)]
pub struct Book {
    pub id: String,
    pub title: String,
    pub author: String,
}

#[derive(Clone, Debug)]
pub struct Track {
    pub url: String,
    pub start: f64,
}

#[derive(Clone, Debug)]
pub struct Session {
    pub id: String,
    pub start: f64,
    pub duration: f64,
    pub tracks: Vec<Track>,
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
        let libs = self.call(reqwest::Method::GET, "/api/libraries", None)?;
        let mut out = vec![];
        for l in libs["libraries"].as_array().into_iter().flatten().filter(|l| l["mediaType"] != "podcast") {
            let r = self.call(reqwest::Method::GET, &format!("/api/libraries/{}/items?limit=100&sort=media.metadata.title", s(&l["id"])), None)?;
            for i in r["results"].as_array().into_iter().flatten() {
                let md = &i["media"]["metadata"];
                out.push(Book { id: s(&i["id"]), title: s(&md["title"]), author: md["authorName"].as_str().or(md["author"].as_str()).unwrap_or_default().into() });
            }
        }
        Ok(out)
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
        Ok(Session { id: s(&v["id"]), start: v["currentTime"].as_f64().unwrap_or(0.0), duration: v["duration"].as_f64().unwrap_or(0.0), tracks })
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
