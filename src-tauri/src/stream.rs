//! Loopback streaming for video and audio previews of providers without presigned URLs (WebDAV,
//! OneDrive, Box, SFTP), ADR-052.
//!
//! `nxr://` (proxy.rs) must answer with a whole body, so a player waits for every piece to
//! download before it gets a byte of it, and a video from a NAS took seconds to start where an S3
//! link plays at once. Here the player gets a plain `http://127.0.0.1:<port>/<token>/…` URL, as it
//! does from S3, and bytes are passed on as they arrive, with the account's login added by the app.
//!
//! The server starts with the first media preview and listens on 127.0.0.1 only. The path carries
//! a random token made at start, so other programs on this computer can't read through it. It
//! serves GET and HEAD with `Range`, one request per connection.

use std::net::Ipv4Addr;

use percent_encoding::percent_decode_str;
use std::sync::Arc;

use tauri::{AppHandle, Manager};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::OnceCell;

use crate::provider::Remote;
use crate::state::AppState;

/// An account's `Remote` by id (the app's accounts; a stand-in in tests).
type Lookup = Arc<dyn Fn(&str) -> Result<Arc<Remote>, String> + Send + Sync>;

/// Pieces read from providers without an open ranged GET (SFTP).
const PIECE: u64 = 1024 * 1024;

static SERVER: OnceCell<(u16, String)> = OnceCell::const_new();

/// The streaming URL for a preview URL from `proxy::url` (same account/container/key path).
pub async fn url(app: &AppHandle, proxy_url: &str) -> Result<String, String> {
    let path = proxy_url.strip_prefix(crate::proxy::base()).ok_or("Not a preview URL")?;
    let (port, token) = SERVER
        .get_or_try_init(|| {
            let app = app.clone();
            start(Arc::new(move |account: &str| app.state::<AppState>().remote(Some(account))))
        })
        .await?;
    Ok(format!("http://127.0.0.1:{port}/{token}{path}"))
}

async fn start(lookup: Lookup) -> Result<(u16, String), String> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let token = uuid::Uuid::new_v4().simple().to_string();
    let t = token.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else { continue };
            let (lookup, t) = (lookup.clone(), t.clone());
            tauri::async_runtime::spawn(async move {
                let _ = serve(&lookup, sock, &t).await; // a player that hangs up mid-file is normal
            });
        }
    });
    Ok((port, token))
}

/// The request line and `Range` of one request.
struct Req {
    head: bool,
    path: String,
    range: Option<(u64, Option<u64>)>,
}

async fn read_req(sock: &mut BufReader<TcpStream>) -> Option<Req> {
    let mut line = String::new();
    sock.read_line(&mut line).await.ok()?;
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next()?.to_string(), parts.next()?.to_string());
    let mut range = None;
    for _ in 0..100 {
        let mut h = String::new();
        if sock.read_line(&mut h).await.ok()? == 0 || h.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = h.split_once(':')
            && name.trim().eq_ignore_ascii_case("range")
        {
            range = crate::proxy::parse_range(value.trim());
        }
    }
    matches!(method.as_str(), "GET" | "HEAD").then_some(Req { head: method == "HEAD", path, range })
}

/// Same-length comparison that doesn't stop at the first difference.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0, |d, (x, y)| d | (x ^ y)) == 0
}

/// What goes back to the player: status, `Content-Range`, length, and how much of the upstream
/// body to skip first (a server that ignored `Range` sends from byte 0).
#[derive(Debug, PartialEq)]
struct Plan {
    partial: bool,
    first: u64,
    last: Option<u64>,
    total: Option<u64>,
    skip: u64,
}

impl Plan {
    /// From the upstream answer: 206 with `Content-Range`, or 200 with the whole file.
    fn of(asked: Option<(u64, Option<u64>)>, status: u16, content_range: Option<&str>, len: Option<u64>) -> Plan {
        if status == 206 {
            // "bytes 100-199/1000"
            let cr = content_range.and_then(|s| s.strip_prefix("bytes ")).unwrap_or("");
            let (span, total) = cr.split_once('/').unwrap_or(("", "*"));
            let (a, b) = span.split_once('-').unwrap_or(("", ""));
            let first = a.parse().unwrap_or(asked.map_or(0, |r| r.0));
            let last = b.parse().ok().or_else(|| len.map(|l| first + l - 1));
            return Plan { partial: true, first, last, total: total.parse().ok(), skip: 0 };
        }
        match (asked, len) {
            (Some((start, end)), Some(total)) if total > 0 => {
                let last = end.unwrap_or(total - 1).min(total - 1);
                Plan { partial: true, first: start, last: Some(last), total: Some(total), skip: start }
            }
            _ => Plan { partial: false, first: 0, last: len.and_then(|l| l.checked_sub(1)), total: len, skip: 0 },
        }
    }

    fn len(&self) -> Option<u64> {
        self.last.map(|l| (l + 1).saturating_sub(self.first))
    }

    fn head(&self, mime: &str) -> String {
        let mut h = format!(
            "HTTP/1.1 {}\r\nContent-Type: {mime}\r\nAccept-Ranges: bytes\r\nCache-Control: no-store\r\nConnection: close\r\n",
            if self.partial { "206 Partial Content" } else { "200 OK" }
        );
        if let Some(n) = self.len() {
            h += &format!("Content-Length: {n}\r\n");
        }
        if self.partial {
            let total = self.total.map_or("*".into(), |t| t.to_string());
            let last = self.last.map_or("*".into(), |l| l.to_string());
            h += &format!("Content-Range: bytes {}-{last}/{total}\r\n", self.first);
        }
        h + "\r\n"
    }
}

async fn serve(lookup: &Lookup, sock: TcpStream, token: &str) -> std::io::Result<()> {
    let mut sock = BufReader::new(sock);
    let req = tokio::time::timeout(std::time::Duration::from_secs(10), read_req(&mut sock)).await.ok().flatten();
    let mut sock = sock.into_inner();
    let Some(req) = req else {
        return sock.write_all(b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\nContent-Length: 0\r\n\r\n").await;
    };
    let mut parts = req.path.trim_start_matches('/').splitn(4, '/');
    let dec = |s: Option<&str>| percent_decode_str(s.unwrap_or("")).decode_utf8_lossy().into_owned();
    if !same(parts.next().unwrap_or(""), token) {
        return sock.write_all(b"HTTP/1.1 403 Forbidden\r\nConnection: close\r\nContent-Length: 0\r\n\r\n").await;
    }
    let (account, container, key) = (dec(parts.next()), dec(parts.next()), dec(parts.next()));
    let mime = mime_guess::from_path(&key).first_or_octet_stream().to_string();
    if let Err(e) = relay(lookup, &mut sock, &req, &account, &container, &key, &mime).await {
        // Only reached before the head went out: errors after it are the player hanging up.
        let msg = e.replace(['\r', '\n'], " ");
        let res = format!(
            "HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\n\r\n{msg}",
            msg.len()
        );
        let _ = sock.write_all(res.as_bytes()).await;
    }
    Ok(())
}

/// Reads from the provider and writes to the player. `Err` only before anything was written.
async fn relay(
    lookup: &Lookup,
    sock: &mut TcpStream,
    req: &Req,
    account: &str,
    container: &str,
    key: &str,
    mime: &str,
) -> Result<(), String> {
    let remote = lookup(account)?;
    let (start, end) = req.range.unwrap_or((0, None));
    if let Some(mut r) = remote.get_range(key, start, end).await? {
        let cr = r.headers().get(reqwest::header::CONTENT_RANGE).and_then(|v| v.to_str().ok()).map(str::to_string);
        let plan = Plan::of(req.range, r.status().as_u16(), cr.as_deref(), r.content_length());
        if sock.write_all(plan.head(mime).as_bytes()).await.is_err() || req.head {
            return Ok(());
        }
        let (mut skip, mut left) = (plan.skip, plan.len());
        while let Ok(Some(chunk)) = r.chunk().await {
            let from = skip.min(chunk.len() as u64) as usize;
            skip -= from as u64;
            let take = left.map_or(chunk.len() - from, |l| (chunk.len() - from).min(l as usize));
            if take > 0 && sock.write_all(&chunk[from..from + take]).await.is_err() {
                break;
            }
            left = left.map(|l| l - take as u64);
            if left == Some(0) {
                break;
            }
        }
        return Ok(());
    }
    // No open GET (SFTP): read pieces in turn.
    let (first, total) = remote.read_range(container, key, start, end.map(|e| e.min(start + PIECE - 1)), PIECE).await?;
    let total = total.ok_or("The server didn't say how big the file is")?;
    let plan = Plan::of(req.range.or(Some((0, None))), 200, None, Some(total));
    let plan = Plan { partial: req.range.is_some(), skip: 0, ..plan };
    if sock.write_all(plan.head(mime).as_bytes()).await.is_err() || req.head {
        return Ok(());
    }
    let Some(last) = plan.last else { return Ok(()) }; // an empty file
    let (mut at, mut piece) = (start, first);
    loop {
        let keep = piece.len().min((last + 1).saturating_sub(at) as usize);
        if keep == 0 || sock.write_all(&piece[..keep]).await.is_err() {
            return Ok(());
        }
        at += keep as u64;
        if at > last {
            return Ok(());
        }
        piece = match remote.read_range(container, key, at, Some(last.min(at + PIECE - 1)), PIECE).await {
            Ok((p, _)) => p,
            Err(_) => return Ok(()), // the head is out; the player sees a short body and asks again
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_follow_the_upstream_answer() {
        // 206 passes through.
        let p = Plan::of(Some((100, None)), 206, Some("bytes 100-999/1000"), Some(900));
        assert_eq!(p, Plan { partial: true, first: 100, last: Some(999), total: Some(1000), skip: 0 });
        let h = p.head("video/mp4");
        assert!(h.starts_with("HTTP/1.1 206") && h.contains("Content-Range: bytes 100-999/1000\r\n"));
        assert!(h.contains("Content-Length: 900\r\n"));
        // A server that ignored Range: skip to the start, stop at the end asked for.
        let p = Plan::of(Some((100, Some(199))), 200, None, Some(1000));
        assert_eq!(p, Plan { partial: true, first: 100, last: Some(199), total: Some(1000), skip: 100 });
        assert_eq!(p.len(), Some(100));
        // No Range asked: the whole file.
        let p = Plan::of(None, 200, None, Some(1000));
        assert_eq!(p, Plan { partial: false, first: 0, last: Some(999), total: Some(1000), skip: 0 });
        assert!(p.head("video/mp4").starts_with("HTTP/1.1 200 OK"));
    }

    #[test]
    fn tokens_compare_whole() {
        assert!(same("abc", "abc"));
        assert!(!same("abc", "abd"));
        assert!(!same("abc", "ab"));
    }

    /// End to end against a WebDAV server: `NOEXPLORER_TEST_DAV_URL=http://127.0.0.1:8090
    /// NOEXPLORER_TEST_DAV_USER=… NOEXPLORER_TEST_DAV_PASS=… NOEXPLORER_TEST_DAV_KEY=videos/test.mov
    /// cargo test stream_webdav -- --ignored --nocapture`. Prints the URL for players to try.
    #[tokio::test]
    #[ignore]
    async fn stream_webdav() {
        use crate::config::{Profile, ProviderKind};
        let env = |k: &str| std::env::var(k).unwrap();
        let p = Profile {
            id: "dav".into(),
            kind: ProviderKind::Webdav,
            endpoint: env("NOEXPLORER_TEST_DAV_URL"),
            access_key: env("NOEXPLORER_TEST_DAV_USER"),
            secret_key: env("NOEXPLORER_TEST_DAV_PASS"),
            ..Default::default()
        };
        let remote = Arc::new(Remote::new(&p));
        let (port, token) = start(Arc::new(move |_: &str| Ok(remote.clone()))).await.unwrap();
        let key = env("NOEXPLORER_TEST_DAV_KEY");
        let url = format!("http://127.0.0.1:{port}/{token}/dav/Files/{key}");
        println!("STREAM {url}");
        let http = crate::webdav::http();
        let r = http.get(&url).header("Range", "bytes=0-1").send().await.unwrap();
        assert_eq!(r.status(), 206);
        assert_eq!(r.bytes().await.unwrap().len(), 2);
        let bad = format!("http://127.0.0.1:{port}/wrong/dav/Files/{key}");
        assert_eq!(http.get(&bad).send().await.unwrap().status(), 403);
        let secs: u64 = std::env::var("NOEXPLORER_TEST_STREAM_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
        tokio::time::sleep(std::time::Duration::from_secs(secs)).await; // time for an outside player
    }
}
