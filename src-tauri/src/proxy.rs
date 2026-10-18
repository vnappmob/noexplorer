//! `nxr://` — streams remote files to the webview for previews when a provider has no presigned
//! URLs (WebDAV needs an auth header that `<img>`/`<video>` can't send). ADR-026.
//!
//! URL: `nxr://localhost/<account>/<container>/<key>` (`http://nxr.localhost/…` on Windows, where
//! WebView2 serves custom schemes over http). Honors `Range` so video can seek: ranged answers are
//! capped at [`RANGE_CAP`] (the player asks for the rest), whole-file answers at [`WHOLE_CAP`].
//!
//! Local files come through here too (`nxr://localhost/~local/f/<path>`, built in `files.ts`),
//! instead of Tauri's asset protocol: that one answers from a worker thread, see [`respond`].

use percent_encoding::percent_decode_str;
use tauri::http::{Request, Response, StatusCode, header};
use tauri::{AppHandle, Manager};

use crate::state::AppState;

const RANGE_CAP: u64 = 8 * 1024 * 1024;
const WHOLE_CAP: u64 = 512 * 1024 * 1024;
/// The account segment of local-file URLs (account ids are UUIDs, so it can't collide).
const LOCAL: &str = "~local";

/// Answers a request on the main thread. WebKit stops a request there when its `<img>`/`<video>`
/// goes away (the grid drops tiles as it scrolls), and wry only checks "still running?" right
/// before it answers. Answered from a worker, a request stopped in between makes WebKit throw an
/// Objective-C exception, and with `panic = "abort"` that kills the app (crash in 0.4.12). On the
/// main thread the check and the answer can't be split by a stop.
pub fn respond(app: &AppHandle, responder: tauri::UriSchemeResponder, res: Response<Vec<u8>>) {
    let _ = app.run_on_main_thread(move || responder.respond(res));
}

/// The preview URL for a remote file.
pub fn url(account: &str, container: &str, key: &str) -> String {
    let base = if cfg!(windows) { "http://nxr.localhost" } else { "nxr://localhost" };
    let enc = crate::webdav::enc;
    let key = key.split('/').map(enc).collect::<Vec<_>>().join("/");
    format!("{base}/{}/{}/{key}", enc(account), enc(container))
}

/// `bytes=a-b` / `bytes=a-` → (a, Some(b) / None).
fn parse_range(v: &str) -> Option<(u64, Option<u64>)> {
    let (a, b) = v.strip_prefix("bytes=")?.split_once('-')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()))
}

pub async fn handle(app: AppHandle, req: Request<Vec<u8>>) -> Response<Vec<u8>> {
    // The page's origin differs from nxr://, so fetch() needs CORS (a Range header preflights).
    if req.method() == tauri::http::Method::OPTIONS {
        return Response::builder()
            .status(StatusCode::NO_CONTENT)
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET, HEAD, OPTIONS")
            .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "Range")
            .body(vec![])
            .unwrap();
    }
    match serve(&app, &req).await {
        Ok(r) => r,
        Err(e) => Response::builder()
            .status(StatusCode::BAD_GATEWAY)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(e.into_bytes())
            .unwrap(),
    }
}

async fn serve(app: &AppHandle, req: &Request<Vec<u8>>) -> Result<Response<Vec<u8>>, String> {
    let path = req.uri().path().trim_start_matches('/');
    let mut parts = path.splitn(3, '/');
    let dec = |s: Option<&str>| percent_decode_str(s.unwrap_or("")).decode_utf8_lossy().into_owned();
    let (account, container, key) = (dec(parts.next()), dec(parts.next()), dec(parts.next()));
    let range = req.headers().get(header::RANGE).and_then(|v| v.to_str().ok()).and_then(parse_range);
    let (start, end, cap) = match range {
        Some((a, b)) => (a, b, RANGE_CAP),
        None => (0, None, WHOLE_CAP),
    };
    let (bytes, total) = if account == LOCAL {
        read_local(&key, start, end, cap).await?
    } else {
        app.state::<AppState>().remote(Some(&account))?.read_range(&container, &key, start, end, cap).await?
    };
    let mime = mime_guess::from_path(&key).first_or_octet_stream().to_string();
    let mime = if mime.starts_with("text/") || crate::util::is_text_like(&key) {
        format!("{}; charset=utf-8", if mime.starts_with("text/") { mime.as_str() } else { "text/plain" })
    } else {
        mime
    };
    // No Content-Length here: the webview adds it from the body, and a duplicate breaks media loading.
    let mut res = Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::ACCESS_CONTROL_EXPOSE_HEADERS, "Content-Range, Content-Length, Accept-Ranges");
    res = if range.is_some() && !bytes.is_empty() {
        let last = start + bytes.len() as u64 - 1;
        let total = total.map(|t| t.to_string()).unwrap_or_else(|| "*".into());
        res.status(StatusCode::PARTIAL_CONTENT).header(header::CONTENT_RANGE, format!("bytes {start}-{last}/{total}"))
    } else {
        res.status(StatusCode::OK)
    };
    res.body(bytes).map_err(|e| e.to_string())
}

/// Up to `cap` bytes of a local file from `start` (to `end` inclusive), plus its size.
async fn read_local(path: &str, start: u64, end: Option<u64>, cap: u64) -> Result<(Vec<u8>, Option<u64>), String> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let mut f = tokio::fs::File::open(path).await.map_err(|e| e.to_string())?;
    let total = f.metadata().await.map_err(|e| e.to_string())?.len();
    let last = end.unwrap_or(u64::MAX).min(start.saturating_add(cap) - 1).min(total.saturating_sub(1));
    if start >= total {
        return Ok((vec![], Some(total)));
    }
    f.seek(std::io::SeekFrom::Start(start)).await.map_err(|e| e.to_string())?;
    let mut buf = Vec::with_capacity((last - start + 1) as usize);
    f.take(last - start + 1).read_to_end(&mut buf).await.map_err(|e| e.to_string())?;
    Ok((buf, Some(total)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_and_ranges() {
        let u = url("id-1", "Files", "a b/c&d.mp4");
        assert!(u.ends_with("/id-1/Files/a%20b/c&d.mp4"), "{u}");
        assert_eq!(parse_range("bytes=0-"), Some((0, None)));
        assert_eq!(parse_range("bytes=100-199"), Some((100, Some(199))));
        assert_eq!(parse_range("items=1-2"), None);
    }

    #[test]
    fn local_ranges() {
        let dir = std::env::temp_dir().join(format!("noexplorer-ut-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.bin");
        std::fs::write(&p, b"0123456789").unwrap();
        let p = p.to_string_lossy().to_string();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        assert_eq!(rt.block_on(read_local(&p, 0, None, 100)).unwrap(), (b"0123456789".to_vec(), Some(10)));
        assert_eq!(rt.block_on(read_local(&p, 2, Some(4), 100)).unwrap().0, b"234");
        assert_eq!(rt.block_on(read_local(&p, 8, Some(99), 100)).unwrap().0, b"89");
        assert_eq!(rt.block_on(read_local(&p, 3, None, 2)).unwrap().0, b"34");
        assert!(rt.block_on(read_local(&p, 10, None, 100)).unwrap().0.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
