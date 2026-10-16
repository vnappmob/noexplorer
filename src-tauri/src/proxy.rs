//! `nxr://` — streams remote files to the webview for previews when a provider has no presigned
//! URLs (WebDAV needs an auth header that `<img>`/`<video>` can't send). ADR-026.
//!
//! URL: `nxr://localhost/<account>/<container>/<key>` (`http://nxr.localhost/…` on Windows, where
//! WebView2 serves custom schemes over http). Honors `Range` so video can seek: ranged answers are
//! capped at [`RANGE_CAP`] (the player asks for the rest), whole-file answers at [`WHOLE_CAP`].

use percent_encoding::percent_decode_str;
use tauri::http::{Request, Response, StatusCode, header};
use tauri::{AppHandle, Manager};

use crate::state::AppState;

const RANGE_CAP: u64 = 8 * 1024 * 1024;
const WHOLE_CAP: u64 = 512 * 1024 * 1024;

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
    let remote = app.state::<AppState>().remote(Some(&account))?;
    let range = req.headers().get(header::RANGE).and_then(|v| v.to_str().ok()).and_then(parse_range);
    let (start, end, cap) = match range {
        Some((a, b)) => (a, b, RANGE_CAP),
        None => (0, None, WHOLE_CAP),
    };
    let (bytes, total) = remote.read_range(&container, &key, start, end, cap).await?;
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
}
