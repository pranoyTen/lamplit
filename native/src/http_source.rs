// A remote audio file as a seekable byte stream: reads are served from the current block, and a read outside it fetches the next
// block with an HTTP Range request. Runs on the decode thread, never on the audio thread, so a slow network only drains the buffer.
use std::io::{self, Read, Seek, SeekFrom};

const BLOCK: u64 = 512 * 1024;

pub struct HttpSource {
    http: reqwest::blocking::Client,
    token: String,
    url: String,
    pub len: u64,
    pub mime: Option<String>,
    pos: u64,
    block_start: u64,
    block: Vec<u8>,
}

/// "bytes 0-1023/52428800" -> 52428800
pub fn total_len(content_range: &str) -> Option<u64> {
    content_range.rsplit('/').next()?.trim().parse().ok()
}

impl HttpSource {
    pub fn open(http: reqwest::blocking::Client, token: String, url: String) -> io::Result<Self> {
        let mut s = HttpSource { http, token, url, len: 0, mime: None, pos: 0, block_start: 0, block: vec![] };
        s.fetch(0)?;
        Ok(s)
    }

    fn fetch(&mut self, at: u64) -> io::Result<()> {
        let end = at + BLOCK - 1;
        let res = self.http.get(&self.url).bearer_auth(&self.token).header("Range", format!("bytes={at}-{end}")).send().map_err(io::Error::other)?;
        let status = res.status().as_u16();
        if status != 206 && status != 200 {
            return Err(io::Error::other(format!("the server answered {status} for the audio")));
        }
        if self.len == 0 {
            let cr = res.headers().get("content-range").and_then(|v| v.to_str().ok()).and_then(total_len);
            self.len = cr.or(res.content_length()).unwrap_or(0);
            self.mime = res.headers().get("content-type").and_then(|v| v.to_str().ok()).map(str::to_string);
        }
        // shortcut: a server that ignores Range (200) sends the whole file into memory; Audiobookshelf honours Range
        let start = if status == 200 { 0 } else { at };
        self.block = res.bytes().map_err(io::Error::other)?.to_vec();
        self.block_start = start;
        Ok(())
    }
}

impl Read for HttpSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.len {
            return Ok(0);
        }
        let in_block = self.pos >= self.block_start && self.pos < self.block_start + self.block.len() as u64;
        if !in_block {
            self.fetch(self.pos)?;
        }
        let off = (self.pos - self.block_start) as usize;
        let n = buf.len().min(self.block.len().saturating_sub(off));
        buf[..n].copy_from_slice(&self.block[off..off + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for HttpSource {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let p = match to {
            SeekFrom::Start(p) => p as i64,
            SeekFrom::End(d) => self.len as i64 + d,
            SeekFrom::Current(d) => self.pos as i64 + d,
        };
        if p < 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "seek before the start"));
        }
        self.pos = p as u64;
        Ok(self.pos)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn content_range_total() {
        assert_eq!(super::total_len("bytes 0-1023/52428800"), Some(52428800));
        assert_eq!(super::total_len("bytes */100"), Some(100));
        assert_eq!(super::total_len("bytes 0-9/*"), None);
    }
}
