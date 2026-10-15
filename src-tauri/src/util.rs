//! Small pure helpers shared by the other modules. Everything here is unit-tested.

use std::path::{Path, PathBuf};

use aws_sdk_s3::error::DisplayErrorContext;

/// Turns an SDK error into a short, human message: "<S3 message> (<Code>). <hint>".
/// The SDK's plain `Display` is just "service error", and the full cause chain is noisy, so we
/// pull `code`/`message` out of it, add a hint for common mistakes, and fall back to a
/// truncated chain for anything else.
pub fn err<E: std::error::Error>(e: E) -> String {
    humanize(&format!("{}", DisplayErrorContext(e)))
}

fn field<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let start = s.find(&format!("{name}: Some(\""))? + name.len() + 8;
    let end = s[start..].find("\")")? + start;
    Some(&s[start..end])
}

fn hint(code: &str, message: &str) -> Option<String> {
    Some(match code {
        "AuthorizationHeaderMalformed" | "IllegalLocationConstraintException" => {
            let want = message.split("expecting '").nth(1).and_then(|r| r.split('\'').next());
            match want {
                Some(r) => format!(
                    "This server expects region \"{r}\". Put it in the Region field (for AWS you can leave the endpoint empty)."
                ),
                None => "The Region doesn't match this server.".into(),
            }
        }
        "PermanentRedirect" => "The bucket lives in another region/endpoint. For AWS, leave the endpoint empty.".into(),
        "SignatureDoesNotMatch" => "Check the secret key (and that the endpoint/region are right).".into(),
        "InvalidAccessKeyId" => "Check the access key.".into(),
        "AccessDenied" => "These credentials aren't allowed to do this.".into(),
        "NoSuchBucket" => "The bucket doesn't exist, or belongs to another account.".into(),
        "RequestTimeTooSkewed" => "Your computer's clock is wrong; sync it and try again.".into(),
        _ => return None,
    })
}

pub fn humanize(full: &str) -> String {
    if let Some(code) = field(full, "code") {
        let message = field(full, "message").unwrap_or("");
        let mut out = if message.is_empty() { code.to_string() } else { format!("{message} ({code})") };
        if let Some(h) = hint(code, message) {
            out = format!("{out}. {h}");
        }
        return out;
    }
    if full.contains("dispatch failure") || full.contains("io error") {
        // Network problem: the innermost cause is the useful part ("Connection refused (os error 61)").
        let cause = full.rsplit(": ").next().unwrap_or(full).trim();
        return format!("Can't reach the server: {cause}. Check the endpoint and your network.");
    }
    if full.contains("timeout") || full.contains("timed out") {
        return "The server didn't respond in time.".into();
    }
    match full.char_indices().nth(400) {
        Some((i, _)) => format!("{}…", &full[..i]),
        None => full.to_string(),
    }
}

/// Turns an object key (or the part of it below the current prefix) into a relative local
/// path, dropping empty, `.` and `..` segments so a malicious key can't escape the target dir.
pub fn safe_rel(rel: &str) -> PathBuf {
    rel.split('/').filter(|c| !c.is_empty() && *c != "." && *c != "..").collect()
}

/// Returns `p` if free, otherwise `name (1).ext`, `name (2).ext`, … — never overwrites.
pub fn unique_path(p: PathBuf) -> PathBuf {
    if !p.exists() {
        return p;
    }
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let ext = p.extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
    let parent = p.parent().map(Path::to_path_buf).unwrap_or_default();
    (1..).map(|i| parent.join(format!("{stem} ({i}){ext}"))).find(|c| !c.exists()).unwrap()
}

/// Human label for a transfer job: "a.txt", "a.txt, b.txt" or "a.txt, b.txt +3 more".
pub fn job_label(names: Vec<String>) -> String {
    match names.len() {
        0 => String::new(),
        1 | 2 => names.join(", "),
        n => format!("{}, {} +{} more", names[0], names[1], n - 2),
    }
}

/// "1.5 GB", "320 KB" — for tray/notification text (the UI has its own `fmtSize`).
pub fn fmt_bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut v = n as f64;
    let mut i = 0;
    v /= 1024.0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if v < 10.0 { format!("{v:.1} {}", UNITS[i]) } else { format!("{v:.0} {}", UNITS[i]) }
}

/// Last segment of a key or `/`-separated path ("a/b/c.txt" → "c.txt", "a/b/" → "b").
pub fn file_name(key: &str) -> String {
    key.trim_end_matches('/').rsplit('/').next().unwrap_or(key).to_string()
}

/// Extensions previewed as text. Keep in sync with `KIND.text` in `src/files.ts`.
pub fn is_text_like(key: &str) -> bool {
    let ext = key.rsplit('.').next().unwrap_or("").to_lowercase();
    matches!(
        ext.as_str(),
        "txt"
            | "md"
            | "log"
            | "json"
            | "yaml"
            | "yml"
            | "toml"
            | "ini"
            | "conf"
            | "cfg"
            | "env"
            | "csv"
            | "tsv"
            | "xml"
            | "sql"
            | "sh"
            | "bash"
            | "zsh"
            | "py"
            | "js"
            | "ts"
            | "tsx"
            | "jsx"
            | "rs"
            | "go"
            | "java"
            | "kt"
            | "swift"
            | "c"
            | "h"
            | "cpp"
            | "hpp"
            | "cs"
            | "php"
            | "rb"
            | "html"
            | "css"
            | "scss"
            | "vue"
            | "dart"
            | "gradle"
            | "properties"
            | "dockerfile"
            | "gitignore"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_rel_blocks_traversal() {
        assert_eq!(safe_rel("../../etc/passwd"), PathBuf::from("etc/passwd"));
        assert_eq!(safe_rel("a//./b/"), PathBuf::from("a/b"));
        assert_eq!(safe_rel(""), PathBuf::new());
    }

    #[test]
    fn unique_path_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("noexplorer-ut-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.txt");
        assert_eq!(unique_path(p.clone()), p);
        std::fs::write(&p, "x").unwrap();
        assert_eq!(unique_path(p.clone()), dir.join("a (1).txt"));
        std::fs::write(dir.join("a (1).txt"), "x").unwrap();
        assert_eq!(unique_path(p), dir.join("a (2).txt"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn labels_and_names() {
        assert_eq!(job_label(vec!["a".into()]), "a");
        assert_eq!(job_label(vec!["a".into(), "b".into(), "c".into(), "d".into()]), "a, b +2 more");
        assert_eq!(file_name("x/y/z.bin"), "z.bin");
        assert_eq!(
            (fmt_bytes(512), fmt_bytes(1536), fmt_bytes(5 << 30)),
            ("512 B".into(), "1.5 KB".into(), "5.0 GB".into())
        );
        assert_eq!(file_name("x/y/"), "y");
        assert!(is_text_like("README.MD"));
        assert!(!is_text_like("photo.jpg"));
    }

    #[test]
    fn humanizes_s3_errors() {
        let raw = r#"service error: unhandled error (AuthorizationHeaderMalformed): Error { code: "AuthorizationHeaderMalformed", message: "The authorization header is malformed; the region 'ap-southeast-1' is wrong; expecting 'us-east-1'", aws_request_id: "X" } (ServiceError(ServiceError { source: Unhandled(Unhandled { source: ErrorMetadata { code: Some("AuthorizationHeaderMalformed"), message: Some("The authorization header is malformed; the region 'ap-southeast-1' is wrong; expecting 'us-east-1'"), extras: None } }) }))"#;
        let h = humanize(raw);
        assert!(h.starts_with("The authorization header is malformed"), "{h}");
        assert!(h.contains("expects region \"us-east-1\""), "{h}");
        assert!(!h.contains("ServiceError"), "{h}");

        let sig = r#"service error: unhandled error (SignatureDoesNotMatch): Error { code: Some("SignatureDoesNotMatch"), message: Some("The request signature we calculated does not match") }"#;
        assert_eq!(
            humanize(sig),
            "The request signature we calculated does not match (SignatureDoesNotMatch). Check the secret key (and that the endpoint/region are right)."
        );

        let net =
            "dispatch failure: io error: error trying to connect: tcp connect error: Connection refused (os error 61)";
        assert_eq!(
            humanize(net),
            "Can't reach the server: Connection refused (os error 61). Check the endpoint and your network."
        );
    }

    #[test]
    fn err_truncates_on_char_boundary() {
        #[derive(Debug)]
        struct E(String);
        impl std::fmt::Display for E {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
        impl std::error::Error for E {}
        let s = err(E("ồ".repeat(1000)));
        assert!(s.ends_with('…') && s.chars().count() == 401);
    }
}
