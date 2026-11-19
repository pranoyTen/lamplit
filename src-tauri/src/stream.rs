// Relays streamed audio from the Audiobookshelf server to the webview with a CORS header. The equalizer runs the audio through Web Audio,
// which plays silence for another origin's audio unless that origin allows it, and the server only allows its own apps by default.
// The webview asks for byte ranges; each answer is at most CHUNK bytes, so a whole book is never held in memory.
use std::sync::OnceLock;
use tauri::http::header::{ACCEPT_RANGES, ACCESS_CONTROL_ALLOW_ORIGIN, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE};
use tauri::http::{Request, Response, StatusCode};

const CHUNK: u64 = 2 * 1024 * 1024;

fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(reqwest::Client::new)
}

/// The range to ask the server for: what the webview asked ("bytes=START-" or "bytes=START-END"), capped at CHUNK bytes.
pub fn range(h: Option<&str>) -> (u64, u64) {
    let (s, e) = h.and_then(|h| h.strip_prefix("bytes=")).and_then(|r| r.split_once('-')).unwrap_or(("0", ""));
    let s: u64 = s.trim().parse().unwrap_or(0);
    let last = s + CHUNK - 1;
    (s, e.trim().parse().map_or(last, |e: u64| e.clamp(s, last)))
}

pub async fn proxy(req: Request<Vec<u8>>) -> Response<Vec<u8>> {
    fetch(&req).await.unwrap_or_else(|e| {
        Response::builder().status(StatusCode::BAD_GATEWAY).header(ACCESS_CONTROL_ALLOW_ORIGIN, "*").body(e.into_bytes()).unwrap()
    })
}

async fn fetch(req: &Request<Vec<u8>>) -> Result<Response<Vec<u8>>, String> {
    // the real address is the whole path, percent-encoded (convertFileSrc(url, 'stream') in the frontend)
    let url = percent_encoding::percent_decode_str(req.uri().path().trim_start_matches('/')).decode_utf8().map_err(|e| e.to_string())?;
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(format!("not a web address: {url}"));
    }
    let (s, e) = range(req.headers().get(RANGE).and_then(|v| v.to_str().ok()));
    // shortcut: a server that ignores Range sends the whole file, which is then held in memory; Audiobookshelf honours Range
    let up = client().get(url.as_ref()).header(RANGE, format!("bytes={s}-{e}")).send().await.map_err(|e| e.to_string())?;
    let mut res = Response::builder().status(up.status().as_u16()).header(ACCESS_CONTROL_ALLOW_ORIGIN, "*").header(ACCEPT_RANGES, "bytes");
    for h in [CONTENT_TYPE, CONTENT_RANGE] {
        if let Some(v) = up.headers().get(&h) {
            res = res.header(h, v.as_bytes());
        }
    }
    let body = up.bytes().await.map_err(|e| e.to_string())?.to_vec();
    res.header(CONTENT_LENGTH, body.len()).body(body).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_are_capped() {
        assert_eq!(range(None), (0, CHUNK - 1));
        assert_eq!(range(Some("bytes=0-")), (0, CHUNK - 1));
        assert_eq!(range(Some("bytes=100-199")), (100, 199));
        assert_eq!(range(Some("bytes=10-99999999")), (10, 10 + CHUNK - 1));
        assert_eq!(range(Some("bytes=50-10")), (50, 50));
    }
}
