//! Password and key-passphrase answers for the system `ssh` (SFTP accounts, `sftp.rs`).
//!
//! `ssh` asks for secrets through the program in `SSH_ASKPASS` when `SSH_ASKPASS_REQUIRE=force`.
//! That program is NoExplorer itself, started with `NOEXPLORER_ASKPASS=<port> <token>`: `main.rs`
//! calls [`run_if_asked`] first, which connects back to the app on 127.0.0.1, sends the one-time
//! token and ssh's prompt, prints the answer for ssh and exits, before any window is created.
//!
//! The secret never goes into an environment variable, a file or a command line: only the running
//! app holds it, and it answers only the child that knows the token, a handful of times, for a
//! minute at most ([`Server`]).

use std::io::{BufRead, BufReader, Read, Write};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

pub const ENV: &str = "NOEXPLORER_ASKPASS";
/// Answers per connection attempt (a passphrase then a password, plus slack), and how long.
const MAX_ANSWERS: usize = 4;
const LIFETIME: Duration = Duration::from_secs(60);

/// Askpass mode: when started by `ssh` with [`ENV`] set, answers its prompt (`argv[1]`) and exits.
/// Returns normally (false) for a normal app launch.
pub fn run_if_asked() -> bool {
    let Ok(v) = std::env::var(ENV) else { return false };
    let prompt = std::env::args().nth(1).unwrap_or_default();
    let code = match ask(&v, &prompt) {
        Ok(answer) if !answer.is_empty() => {
            println!("{answer}");
            0
        }
        _ => 1, // ssh treats a failed askpass as "no answer" and gives up on that method
    };
    std::process::exit(code);
}

fn ask(env: &str, prompt: &str) -> std::io::Result<String> {
    let (port, token) = env.split_once(' ').ok_or(std::io::ErrorKind::InvalidInput)?;
    let port: u16 = port.parse().map_err(|_| std::io::ErrorKind::InvalidInput)?;
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port))?;
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    // One line each: the token, then the prompt (flattened to one line).
    writeln!(s, "{token}\n{}", prompt.replace(['\n', '\r'], " "))?;
    let mut answer = String::new();
    BufReader::new(&mut s).read_line(&mut answer)?;
    let mut rest = Vec::new();
    let _ = s.read_to_end(&mut rest);
    Ok(answer.trim_end_matches(['\n', '\r']).to_string())
}

/// The app side for one `ssh` start: listens on a random loopback port with a random token.
pub struct Server {
    port: u16,
    token: String,
    task: tokio::task::JoinHandle<()>,
}

impl Server {
    pub async fn start(secret: String) -> std::io::Result<Server> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let token = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
        let expect = token.clone();
        let task = tokio::spawn(async move {
            let serve = async {
                for _ in 0..MAX_ANSWERS {
                    let Ok((mut conn, _)) = listener.accept().await else { return };
                    let (r, mut w) = conn.split();
                    let mut lines = tokio::io::BufReader::new(r).lines();
                    let Ok(Some(t)) = lines.next_line().await else { continue };
                    let _prompt = lines.next_line().await;
                    // A wrong token gets nothing (constant-time compare is moot: random 256-bit
                    // token, loopback only, a few tries).
                    if t == expect {
                        let _ = w.write_all(format!("{secret}\n").as_bytes()).await;
                    }
                    let _ = w.shutdown().await;
                }
            };
            let _ = tokio::time::timeout(LIFETIME, serve).await;
        });
        Ok(Server { port, token, task })
    }

    /// The value of [`ENV`] for the `ssh` child.
    pub fn env_value(&self) -> String {
        format!("{} {}", self.port, self.token)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn answers_only_with_the_token() {
        let srv = Server::start("s3cr3t pass".into()).await.unwrap();
        let env = srv.env_value();
        let ok = tokio::task::spawn_blocking(move || ask(&env, "Enter passphrase for key:")).await.unwrap();
        assert_eq!(ok.unwrap(), "s3cr3t pass");
        let wrong = format!("{} {}", srv.port, "0".repeat(64));
        let bad = tokio::task::spawn_blocking(move || ask(&wrong, "Password:")).await.unwrap();
        assert_eq!(bad.unwrap(), "", "a wrong token gets no answer");
        assert!(ask("not a port", "x").is_err());
    }
}
