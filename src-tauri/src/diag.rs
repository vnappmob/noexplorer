//! Diagnostic log: every failed request to a storage server is appended to
//! `<OS log folder>/noexplorer.log` (macOS: `~/Library/Logs/<bundle id>/`), with what the server
//! said, so a user can send it when something fails. Settings → About → "Open log" reveals it.
//!
//! Nothing secret goes in: signatures, credentials, access key ids and tokens are redacted
//! ([`redact`]); secret keys never appear in these messages to begin with. The file stays small
//! (trimmed to its last half past [`MAX`]).

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Manager};

const MAX: u64 = 1 << 20;
/// Longest single entry kept (server answers can be long HTML pages).
const ENTRY_MAX: usize = 4000;

static FILE: OnceLock<PathBuf> = OnceLock::new();
static LOCK: Mutex<()> = Mutex::new(());

pub fn init(app: &AppHandle) {
    if let Ok(d) = app.path().app_log_dir() {
        let _ = FILE.set(d.join("noexplorer.log"));
    }
}

/// Appends one entry: `<UTC time> [<source>] <detail>`.
pub fn log(source: &str, detail: &str) {
    let Some(path) = FILE.get() else { return };
    let mut text = redact(detail);
    if let Some((i, _)) = text.char_indices().nth(ENTRY_MAX) {
        text.truncate(i);
        text.push('…');
    }
    let line = format!("{} [{source}] {}\n", utc_now(), text.replace('\n', "\\n"));
    let _guard = LOCK.lock().unwrap();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::metadata(path).is_ok_and(|m| m.len() > MAX)
        && let Ok(old) = std::fs::read(path)
    {
        let keep = &old[old.len() / 2..];
        let start = keep.iter().position(|&b| b == b'\n').map_or(0, |i| i + 1);
        let _ = std::fs::write(path, &keep[start..]);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(line.as_bytes());
    }
}

/// The log file's path (created empty if missing), for the "Open log" button.
#[tauri::command]
pub fn log_file() -> Result<String, String> {
    let path = FILE.get().ok_or("No log folder on this system")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

/// Blanks out anything that could identify or replay a credential.
pub fn redact(s: &str) -> String {
    let mut out = s.to_string();
    // Query parameters of presigned / SAS links, and header-style "key=value" pairs.
    for key in ["X-Amz-Signature=", "X-Amz-Credential=", "X-Amz-Security-Token=", "sig=", "Signature=", "Credential="] {
        out = blank_after(&out, key, |c| c == '&' || c == ',' || c == ' ' || c == '"' || c == '\'' || c == '<');
    }
    // Server error bodies (AWS echoes the key id and the signature it got).
    for tag in ["AWSAccessKeyId", "SignatureProvided", "HostId", "AccessKeyId"] {
        out = blank_between(&out, &format!("<{tag}>"), &format!("</{tag}>"));
    }
    for key in ["Bearer ", "Basic ", "access_token\":\"", "refresh_token\":\""] {
        out = blank_after(&out, key, |c| c == ' ' || c == '"' || c == ',' || c == '\\' || c == '}');
    }
    out
}

fn blank_after(s: &str, key: &str, stop: impl Fn(char) -> bool) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(key) {
        out.push_str(&rest[..i + key.len()]);
        rest = &rest[i + key.len()..];
        let end = rest.find(&stop).unwrap_or(rest.len());
        if end > 0 {
            out.push_str("REDACTED");
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

fn blank_between(s: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(open) {
        out.push_str(&rest[..i + open.len()]);
        rest = &rest[i + open.len()..];
        let end = rest.find(close).unwrap_or(rest.len());
        out.push_str("REDACTED");
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// `2026-10-19T07:02:44Z` (UTC; no date crate needed).
fn utc_now() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_credentials_but_keeps_the_reason() {
        let s = r#"<Error><Code>SignatureDoesNotMatch</Code><AWSAccessKeyId>AKIAEXAMPLE</AWSAccessKeyId><SignatureProvided>abc123</SignatureProvided><Details>Check your key.</Details></Error> https://h/b/k?X-Amz-Credential=AKIA%2F2026&X-Amz-Signature=deadbeef&x=1 Authorization: Bearer tok.en.value sig=ZZZ== {"access_token":"secret1","x":1}"#;
        let r = redact(s);
        for gone in ["AKIAEXAMPLE", "abc123", "AKIA%2F2026", "deadbeef", "tok.en.value", "ZZZ==", "secret1"] {
            assert!(!r.contains(gone), "{gone} leaked: {r}");
        }
        assert!(r.contains("SignatureDoesNotMatch") && r.contains("Check your key.") && r.contains("&x=1"), "{r}");
    }

    #[test]
    fn utc_dates() {
        let t = utc_now();
        assert_eq!(t.len(), 20, "{t}");
        assert!(t.starts_with("20") && t.ends_with('Z'), "{t}");
    }
}
