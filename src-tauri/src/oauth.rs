//! Small helpers for browser sign-ins and JSON REST providers (OneDrive): the loopback listener
//! that waits for the OAuth redirect, form/query encoding, and RFC 3339 timestamps.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Accepts connections on the loopback listener until the provider (`who`) redirects back to
/// `path` (e.g. `/callback?`) with `code`.
pub(crate) async fn wait_for_code(
    listener: &tokio::net::TcpListener,
    csrf: &str,
    path: &str,
    who: &str,
) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(15 * 60);
    loop {
        if crate::webdav::LOGIN_CANCEL.load(Ordering::Relaxed) {
            return Err("Sign-in cancelled".into());
        }
        if Instant::now() > deadline {
            return Err("Sign-in timed out. Try again.".into());
        }
        let Ok(Ok((mut sock, _))) = tokio::time::timeout(Duration::from_secs(1), listener.accept()).await else {
            continue;
        };
        let mut buf = vec![0u8; 8192];
        let n = tokio::time::timeout(Duration::from_secs(5), sock.read(&mut buf))
            .await
            .ok()
            .and_then(|r| r.ok())
            .unwrap_or(0);
        let head = String::from_utf8_lossy(&buf[..n]).to_string();
        let target = head.split_whitespace().nth(1).unwrap_or("").to_string();
        let Some(query) = target.strip_prefix(path) else {
            let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            continue;
        };
        let params = parse_query(query);
        let (msg, result) = if params.get("state").map(String::as_str) != Some(csrf) {
            (
                "This sign-in link doesn't match. Start again from NoExplorer.".to_string(),
                Err("Sign-in didn't match (state)".to_string()),
            )
        } else if let Some(code) = params.get("code") {
            (format!("NoExplorer is connected to {who}. You can close this tab."), Ok(code.clone()))
        } else {
            let why = params.get("error_description").or(params.get("error")).cloned().unwrap_or("denied".into());
            (
                format!("{who} sign-in was not completed. You can close this tab."),
                Err(format!("{who} sign-in failed: {why}")),
            )
        };
        let page = format!(
            "<!doctype html><meta charset=utf-8><title>NoExplorer</title><body style=\"font:16px system-ui;padding:3em;text-align:center\"><p>{msg}</p></body>"
        );
        let _ = sock
            .write_all(
                format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len())
                    .as_bytes(),
            )
            .await;
        return result;
    }
}

pub(crate) fn enc(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

pub(crate) fn form(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{}={}", enc(k), enc(v))).collect::<Vec<_>>().join("&")
}

pub(crate) fn parse_query(q: &str) -> HashMap<String, String> {
    q.split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| {
            let d =
                |s: &str| percent_encoding::percent_decode_str(&s.replace('+', " ")).decode_utf8_lossy().into_owned();
            (d(k), d(v))
        })
        .collect()
}

/// `2026-10-16T10:01:02-07:00` → Unix millis.
pub(crate) fn parse_rfc3339(s: &str) -> Option<i64> {
    let (date, rest) = s.split_once('T')?;
    let mut d = date.split('-').map(|x| x.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()?? as u32, d.next()?? as u32);
    let time = &rest[..8.min(rest.len())];
    let hms: Vec<i64> = time.split(':').filter_map(|x| x.parse().ok()).collect();
    if hms.len() != 3 {
        return None;
    }
    let tz = &rest[8..];
    let tz = tz.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let offset = match tz {
        "" | "Z" => 0,
        t => {
            let sign = if t.starts_with('-') { -1 } else { 1 };
            let hm: Vec<i64> = t[1..].split(':').filter_map(|x| x.parse().ok()).collect();
            sign * (hm.first().copied().unwrap_or(0) * 3600 + hm.get(1).copied().unwrap_or(0) * 60)
        }
    };
    let days = crate::webdav::days_from_civil(y, m, day);
    Some((days * 86_400 + hms[0] * 3600 + hms[1] * 60 + hms[2] - offset) * 1000)
}

/// RFC 3339 timestamp `secs` from now (UTC).
pub(crate) fn rfc3339_in(secs: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() + secs;
    let (y, m, d) = crate::webdav::civil_from_days((now / 86_400) as i64);
    let t = now % 86_400;
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", t / 3600, t % 3600 / 60, t % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:01Z"), Some(1000));
        assert_eq!(parse_rfc3339("1970-01-01T07:00:01+07:00"), Some(1000));
        assert_eq!(parse_rfc3339("2026-10-16T10:01:02-07:00"), parse_rfc3339("2026-10-16T17:01:02Z"));
        let q = parse_query("code=abc%2F1&state=xyz&error_description=a+b");
        assert_eq!((q["code"].as_str(), q["state"].as_str(), q["error_description"].as_str()), ("abc/1", "xyz", "a b"));
        assert_eq!(form(&[("a b", "c/d")]), "a%20b=c%2Fd");
        assert!(rfc3339_in(86_400).ends_with('Z'));
    }
}
