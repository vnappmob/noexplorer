//! SFTP provider (experimental `providers`): the system `ssh` command carries the connection, and
//! `russh-sftp` speaks the SFTP protocol over its stdin/stdout.
//!
//! Using `ssh` itself means the user's keys, agent, `~/.ssh/config` and `known_hosts` all apply,
//! and host keys are checked by OpenSSH (`accept-new`: a new server is remembered, a changed key is
//! refused). It runs in batch mode, so there are no password prompts: key or agent login only.
//! `ssh` ships with macOS, Linux and Windows 10+ (OpenSSH Client).
//!
//! The account form maps to: `endpoint` = `sftp://user@host:port/path` (or `host`, `user@host`),
//! `access_key` = user name (optional, overrides the one in the address). One container, "Files",
//! rooted at the address's path (the login folder when there's none).

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use russh_sftp::client::SftpSession;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::config::Profile;
use crate::jobs::Job;
use crate::provider::{BucketInfo, Entry, drop_part, open_part, part_path, resume_offset};
use crate::util::file_name;

/// Bytes per read/write call (the SFTP layer splits them into protocol packets).
const CHUNK: usize = 256 * 1024;

pub struct Sftp {
    profile: Profile,
    host: String,
    port: Option<u16>,
    user: Option<String>,
    /// Root of the container: an absolute path, or "" for the login folder.
    base: String,
    session: tokio::sync::Mutex<Option<Arc<SftpSession>>>,
    /// The `ssh` process behind the session (killed when replaced or dropped).
    child: tokio::sync::Mutex<Option<tokio::process::Child>>,
    /// Tests: an ssh config file (`-F`) with the test key and known_hosts.
    #[cfg(test)]
    pub(crate) ssh_config: Option<std::path::PathBuf>,
}

impl Sftp {
    pub fn new(p: &Profile) -> Sftp {
        let raw = p.endpoint.trim();
        let url =
            reqwest::Url::parse(&if raw.contains("://") { raw.to_string() } else { format!("sftp://{raw}") }).ok();
        let host = url.as_ref().and_then(|u| u.host_str()).unwrap_or(raw).to_string();
        let port = url.as_ref().and_then(|u| u.port());
        let user = Some(p.access_key.trim().to_string())
            .filter(|u| !u.is_empty())
            .or_else(|| url.as_ref().map(|u| u.username().to_string()).filter(|u| !u.is_empty()));
        let base = url
            .as_ref()
            .map(|u| {
                percent_encoding::percent_decode_str(u.path()).decode_utf8_lossy().trim_end_matches('/').to_string()
            })
            .unwrap_or_default();
        Sftp {
            profile: p.clone(),
            host,
            port,
            user,
            base,
            session: tokio::sync::Mutex::new(None),
            child: tokio::sync::Mutex::new(None),
            #[cfg(test)]
            ssh_config: None,
        }
    }

    pub fn account(&self) -> &str {
        &self.profile.id
    }

    /// Server path of `key` (relative paths are under the login folder).
    fn full(&self, key: &str) -> String {
        let k = key.trim_end_matches('/');
        match (self.base.is_empty(), k.is_empty()) {
            (true, true) => ".".into(),
            (true, false) => k.into(),
            (false, true) => self.base.clone(),
            (false, false) => format!("{}/{k}", self.base),
        }
    }

    async fn connect(&self) -> Result<SftpSession, String> {
        let mut cmd = tokio::process::Command::new("ssh");
        #[cfg(test)]
        if let Some(f) = &self.ssh_config {
            cmd.arg("-F").arg(f);
        }
        cmd.args([
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=15",
            "-o",
            "StrictHostKeyChecking=accept-new",
            "-o",
            "ServerAliveInterval=30",
        ]);
        if let Some(p) = self.port {
            cmd.arg("-p").arg(p.to_string());
        }
        if let Some(u) = &self.user {
            cmd.arg("-l").arg(u);
        }
        // `-s`: run the "sftp" subsystem. Options go before the host (after it, they'd be a command).
        cmd.arg("-s").arg("--").arg(&self.host).arg("sftp");
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flashing up
        let mut child = cmd.spawn().map_err(|e| format!("Couldn't start ssh ({e}). Is OpenSSH installed?"))?;
        let (stdin, stdout, mut stderr) =
            (child.stdin.take().unwrap(), child.stdout.take().unwrap(), child.stderr.take().unwrap());
        let started =
            tokio::time::timeout(Duration::from_secs(25), SftpSession::new(tokio::io::join(stdout, stdin))).await;
        let why = match started {
            Ok(Ok(s)) => {
                *self.child.lock().await = Some(child);
                return Ok(s);
            }
            Ok(Err(e)) => e.to_string(),
            Err(_) => "the server didn't answer in time".to_string(),
        };
        let _ = child.start_kill();
        let mut msg = String::new();
        let _ = tokio::time::timeout(Duration::from_secs(2), stderr.read_to_string(&mut msg)).await;
        // OpenSSH's own notices ("Warning: Permanently added…", "** WARNING: … post-quantum…") aren't
        // the reason; the last other line is (e.g. "Permission denied (publickey).").
        let reason = msg
            .lines()
            .map(str::trim)
            .rfind(|l| !l.is_empty() && !l.starts_with("**") && !l.starts_with("Warning:"))
            .map(|l| format!("SSH: {l}"));
        Err(reason.unwrap_or_else(|| format!("Couldn't open an SFTP session: {why}")))
    }

    /// The open session, connecting on first use (and after a dropped connection).
    async fn sess(&self) -> Result<Arc<SftpSession>, String> {
        let mut g = self.session.lock().await;
        if let Some(s) = g.as_ref() {
            return Ok(s.clone());
        }
        let s = Arc::new(self.connect().await?);
        *g = Some(s.clone());
        Ok(s)
    }

    /// Forgets a broken session; the next call reconnects.
    async fn reset(&self) {
        *self.session.lock().await = None;
        if let Some(mut c) = self.child.lock().await.take() {
            let _ = c.start_kill();
        }
    }

    /// Runs `f` on the session; a connection that died is reopened once.
    async fn with<T, F, Fut>(&self, f: F) -> Result<T, String>
    where
        F: Fn(Arc<SftpSession>) -> Fut,
        Fut: std::future::Future<Output = Result<T, russh_sftp::client::error::Error>>,
    {
        for attempt in 0..2 {
            let s = self.sess().await?;
            match f(s).await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    let lost = matches!(e, russh_sftp::client::error::Error::UnexpectedBehavior(_))
                        || e.to_string().to_lowercase().contains("closed");
                    if lost && attempt == 0 {
                        self.reset().await;
                        continue;
                    }
                    return Err(sftp_err(e));
                }
            }
        }
        unreachable!()
    }

    pub async fn containers(&self) -> Result<Vec<BucketInfo>, String> {
        let root = self.full("");
        self.with(|s| {
            let root = root.clone();
            async move { s.canonicalize(root).await }
        })
        .await?;
        Ok(vec![BucketInfo { name: "Files".into(), region: None }])
    }

    pub async fn list(&self, _container: &str, prefix: &str) -> Result<Vec<Entry>, String> {
        let dir = self.full(prefix);
        let s = self.sess().await?;
        let rd = self
            .with(|s| {
                let dir = dir.clone();
                async move { s.read_dir(dir).await }
            })
            .await?;
        let mut out = vec![];
        for e in rd {
            let name = e.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let mut meta = e.metadata();
            if e.file_type().is_symlink() {
                // Follow links so a link to a folder opens like a folder.
                if let Ok(m) = s.metadata(format!("{dir}/{name}")).await {
                    meta = m;
                }
            }
            let dir_ = meta.is_dir();
            out.push(Entry {
                key: format!("{prefix}{name}{}", if dir_ { "/" } else { "" }),
                name,
                size: if dir_ { 0 } else { meta.len() },
                modified: meta.mtime.map(|t| t as i64 * 1000),
                dir: dir_,
                hash: None,
            });
        }
        Ok(out)
    }

    pub async fn read_range(
        &self,
        key: &str,
        start: u64,
        end: Option<u64>,
        cap: u64,
    ) -> Result<(Vec<u8>, Option<u64>), String> {
        let path = self.full(key);
        let s = self.sess().await?;
        let total = s.metadata(path.clone()).await.map_err(sftp_err)?.len();
        let last = end.unwrap_or(start + cap - 1).min(start + cap - 1).min(total.saturating_sub(1));
        let mut f = s.open(path).await.map_err(sftp_err)?;
        f.seek(std::io::SeekFrom::Start(start)).await.map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; (last + 1).saturating_sub(start) as usize];
        let mut got = 0;
        while got < buf.len() {
            let n = f.read(&mut buf[got..]).await.map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            got += n;
        }
        buf.truncate(got);
        Ok((buf, Some(total)))
    }

    pub async fn read_head(&self, key: &str, max: u64) -> Result<Vec<u8>, String> {
        Ok(self.read_range(key, 0, Some(max.max(1) - 1), max.max(1)).await?.0)
    }

    pub async fn mkdir(&self, _container: &str, prefix: &str, name: &str) -> Result<(), String> {
        let p = self.full(&format!("{prefix}{name}"));
        self.with(|s| {
            let p = p.clone();
            async move { s.create_dir(p).await }
        })
        .await
    }

    /// Creates every missing folder above `key`.
    async fn ensure_parents(&self, key: &str) -> Result<(), String> {
        let s = self.sess().await?;
        let mut acc = String::new();
        let parts: Vec<&str> = key.split('/').collect();
        for p in &parts[..parts.len().saturating_sub(1)] {
            acc.push_str(p);
            acc.push('/');
            let full = self.full(&acc);
            if !s.try_exists(full.clone()).await.unwrap_or(false) {
                s.create_dir(full).await.map_err(sftp_err)?;
            }
        }
        Ok(())
    }

    async fn exists(&self, key: &str) -> Result<bool, String> {
        let p = self.full(key);
        self.with(|s| {
            let p = p.clone();
            async move { s.try_exists(p).await }
        })
        .await
    }

    /// Removes a file, or a folder with everything in it.
    async fn remove_tree(&self, s: &SftpSession, path: &str) -> Result<(), String> {
        let meta = match s.symlink_metadata(path.to_string()).await {
            Ok(m) => m,
            Err(_) => return Ok(()), // already gone
        };
        if !meta.is_dir() {
            return s.remove_file(path.to_string()).await.map_err(sftp_err);
        }
        let mut stack = vec![(path.to_string(), false)];
        while let Some((dir, emptied)) = stack.pop() {
            if emptied {
                s.remove_dir(dir).await.map_err(sftp_err)?;
                continue;
            }
            stack.push((dir.clone(), true));
            for e in s.read_dir(dir.clone()).await.map_err(sftp_err)? {
                let n = e.file_name();
                if n == "." || n == ".." {
                    continue;
                }
                let p = format!("{dir}/{n}");
                if e.file_type().is_dir() {
                    stack.push((p, false))
                } else {
                    s.remove_file(p).await.map_err(sftp_err)?
                }
            }
        }
        Ok(())
    }

    pub async fn delete(&self, _container: &str, keys: Vec<String>, job: Option<&Job>) -> Result<usize, String> {
        let s = self.sess().await?;
        let total = keys.len() as u64;
        if let Some(j) = job {
            j.files_total.store(total, Ordering::Relaxed);
        }
        for (i, k) in keys.iter().enumerate() {
            if let Some(j) = job {
                if j.cancelled() {
                    return Err("cancelled".into());
                }
                j.files_done.store(i as u64, Ordering::Relaxed);
            }
            self.remove_tree(&s, &self.full(k)).await?;
        }
        if let Some(j) = job {
            j.files_done.store(total, Ordering::Relaxed);
        }
        Ok(keys.len())
    }

    /// Copies a file or folder on the server (SFTP has no copy: the bytes pass through the app).
    async fn copy_tree(&self, s: &SftpSession, from: &str, to: &str) -> Result<(), String> {
        let meta = s.metadata(from.to_string()).await.map_err(sftp_err)?;
        if meta.is_dir() {
            s.create_dir(to.to_string()).await.map_err(sftp_err)?;
            for e in s.read_dir(from.to_string()).await.map_err(sftp_err)? {
                let n = e.file_name();
                if n == "." || n == ".." {
                    continue;
                }
                Box::pin(self.copy_tree(s, &format!("{from}/{n}"), &format!("{to}/{n}"))).await?;
            }
            return Ok(());
        }
        let mut src = s.open(from.to_string()).await.map_err(sftp_err)?;
        let mut dst = s.create(to.to_string()).await.map_err(sftp_err)?;
        tokio::io::copy(&mut src, &mut dst).await.map_err(|e| e.to_string())?;
        dst.shutdown().await.map_err(|e| e.to_string())
    }

    pub async fn transfer_within(
        &self,
        _container: &str,
        from: &str,
        to: &str,
        keep: bool,
        job: Option<&Job>,
    ) -> Result<(), String> {
        if self.exists(to).await? {
            return Err(format!("“{}” already exists there", file_name(to)));
        }
        if let Some(j) = job {
            j.files_total.store(1, Ordering::Relaxed);
        }
        self.ensure_parents(to.trim_end_matches('/')).await?;
        let s = self.sess().await?;
        let (a, b) = (self.full(from), self.full(to));
        if keep {
            self.copy_tree(&s, &a, &b).await?
        } else {
            s.rename(a, b).await.map_err(sftp_err)?
        }
        if let Some(j) = job {
            j.files_done.store(1, Ordering::Relaxed);
        }
        Ok(())
    }

    /// Uploads to a temporary name next to the target and renames it into place, so an interrupted
    /// upload never leaves a cut-off file under the real name.
    pub async fn upload_file(
        &self,
        _container: &str,
        key: &str,
        path: &Path,
        _size: u64,
        job: &Job,
    ) -> Result<(), String> {
        self.ensure_parents(key).await?;
        let s = self.sess().await?;
        let target = self.full(key);
        let tmp = {
            let (dir, name) = target
                .rsplit_once('/')
                .map(|(d, n)| (format!("{d}/"), n.to_string()))
                .unwrap_or((String::new(), target.clone()));
            format!("{dir}.{name}.noexplorer-part")
        };
        let mut local = tokio::fs::File::open(path).await.map_err(|e| e.to_string())?;
        let mut remote = s.create(tmp.clone()).await.map_err(sftp_err)?;
        let mut buf = vec![0u8; CHUNK];
        let res: Result<(), String> = async {
            loop {
                if job.cancelled() {
                    return Err("cancelled".into());
                }
                let n = local.read(&mut buf).await.map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                remote.write_all(&buf[..n]).await.map_err(|e| e.to_string())?;
                job.flow(n as u64).await;
            }
            remote.shutdown().await.map_err(|e| e.to_string())
        }
        .await;
        if let Err(e) = res {
            let _ = s.remove_file(tmp).await;
            return Err(e);
        }
        // SFTP v3 rename won't replace: uploads overwrite, like on every other provider.
        if s.try_exists(target.clone()).await.unwrap_or(false) {
            s.remove_file(target.clone()).await.map_err(sftp_err)?;
        }
        s.rename(tmp, target).await.map_err(sftp_err)
    }

    pub async fn download_file(&self, _container: &str, key: &str, path: &Path, job: &Job) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await.map_err(|e| e.to_string())?;
        }
        let s = self.sess().await?;
        let tmp = part_path(path);
        let offset = resume_offset(&tmp, job).await;
        let mut remote = s.open(self.full(key)).await.map_err(sftp_err)?;
        if offset > 0 {
            remote.seek(std::io::SeekFrom::Start(offset)).await.map_err(|e| e.to_string())?;
        }
        let f = open_part(&tmp, offset > 0, offset, job).await?;
        let mut w = tokio::io::BufWriter::with_capacity(1 << 20, f);
        let mut buf = vec![0u8; CHUNK];
        let res: Result<(), String> = async {
            loop {
                if job.cancelled() {
                    return Err("cancelled".into());
                }
                let n = remote.read(&mut buf).await.map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                w.write_all(&buf[..n]).await.map_err(|e| e.to_string())?;
                job.flow(n as u64).await;
            }
            w.flush().await.map_err(|e| e.to_string())
        }
        .await;
        drop(w);
        match res {
            Ok(()) => tokio::fs::rename(&tmp, path).await.map_err(|e| e.to_string()),
            Err(e) => {
                drop_part(&tmp, job, &e).await;
                Err(e)
            }
        }
    }
}

fn sftp_err(e: russh_sftp::client::error::Error) -> String {
    let m = e.to_string();
    let l = m.to_lowercase();
    if l.contains("no such file") {
        "Not found".into()
    } else if l.contains("permission denied") {
        "Permission denied on the server".into()
    } else {
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_addresses() {
        let p =
            |e: &str, u: &str| Sftp::new(&Profile { endpoint: e.into(), access_key: u.into(), ..Default::default() });
        let a = p("sftp://kris@nas.local:2222/volume1/backups/", "");
        assert_eq!(
            (a.host.as_str(), a.port, a.user.as_deref(), a.base.as_str()),
            ("nas.local", Some(2222), Some("kris"), "/volume1/backups")
        );
        assert_eq!(a.full("x/y/"), "/volume1/backups/x/y");
        let b = p("nas.local", "me");
        assert_eq!((b.host.as_str(), b.port, b.user.as_deref(), b.base.as_str()), ("nas.local", None, Some("me"), ""));
        assert_eq!(b.full(""), ".");
        assert_eq!(b.full("a b/c.txt"), "a b/c.txt");
    }

    /// Against an SFTP container, e.g.
    /// `docker run -d -p 2222:22 -v $KEY.pub:/home/demo/.ssh/keys/id.pub:ro atmoz/sftp demo::1001::upload`,
    /// with `NOEXPLORER_TEST_SFTP=sftp://demo@127.0.0.1:2222/upload` and `NOEXPLORER_TEST_SFTP_KEY=$KEY`.
    #[tokio::test]
    async fn sftp_roundtrip() {
        let (Ok(endpoint), Ok(key)) =
            (std::env::var("NOEXPLORER_TEST_SFTP"), std::env::var("NOEXPLORER_TEST_SFTP_KEY"))
        else {
            eprintln!("skipping sftp_roundtrip: NOEXPLORER_TEST_SFTP / _KEY not set");
            return;
        };
        let dir = std::env::temp_dir().join(format!("nx-sftp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("ssh_config");
        std::fs::write(
            &cfg,
            format!(
                "Host *\n  IdentityFile {key}\n  IdentitiesOnly yes\n  UserKnownHostsFile {}\n",
                dir.join("known_hosts").display()
            ),
        )
        .unwrap();
        let mut sf = Sftp::new(&Profile { endpoint, ..Default::default() });
        sf.ssh_config = Some(cfg);
        assert_eq!(sf.containers().await.unwrap()[0].name, "Files");

        let data: Vec<u8> = (0..(3 * 1024 * 1024 + 11)).map(|i| (i * 17 % 251) as u8).collect();
        std::fs::write(dir.join("big.bin"), &data).unwrap();
        std::fs::write(dir.join("a b.txt"), "hello sftp").unwrap();
        let job = Job::untracked("up", "t".into(), "", "Files", "");
        sf.upload_file("Files", "rt/sub/a b.txt", &dir.join("a b.txt"), 10, &job).await.unwrap();
        sf.upload_file("Files", "rt/big.bin", &dir.join("big.bin"), data.len() as u64, &job).await.unwrap();
        sf.mkdir("Files", "rt/", "empty").await.unwrap();
        let level = sf.list("Files", "rt/").await.unwrap();
        assert!(level.iter().any(|e| e.key == "rt/big.bin" && e.size == data.len() as u64));
        assert!(level.iter().any(|e| e.dir && e.key == "rt/sub/"));
        assert!(level.iter().any(|e| e.dir && e.key == "rt/empty/"));
        assert!(!level.iter().any(|e| e.name.ends_with(".noexplorer-part")));
        assert_eq!(sf.read_head("rt/sub/a b.txt", 5).await.unwrap(), b"hello");
        let (part, total) = sf.read_range("rt/big.bin", 10, Some(19), 1 << 20).await.unwrap();
        assert_eq!((part.as_slice(), total), (&data[10..20], Some(data.len() as u64)));

        sf.download_file("Files", "rt/big.bin", &dir.join("out.bin"), &job).await.unwrap();
        assert!(std::fs::read(dir.join("out.bin")).unwrap() == data, "download differs");

        sf.transfer_within("Files", "rt/sub/", "rt/moved/", false, None).await.unwrap();
        sf.transfer_within("Files", "rt/moved/", "rt/copy/", true, None).await.unwrap();
        assert!(sf.transfer_within("Files", "rt/moved/", "rt/copy/", true, None).await.is_err(), "target exists");
        assert_eq!(sf.read_head("rt/copy/a b.txt", 5).await.unwrap(), b"hello");

        sf.delete("Files", vec!["rt/".into()], None).await.unwrap();
        assert!(sf.list("Files", "").await.unwrap().iter().all(|e| e.key != "rt/"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
