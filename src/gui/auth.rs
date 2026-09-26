use serde::{Deserialize, Serialize};
#[cfg(windows)]
use std::net::{TcpListener as LocalListener, TcpStream as LocalStream};
#[cfg(unix)]
use std::os::unix::net::{UnixListener as LocalListener, UnixStream as LocalStream};
#[cfg(unix)]
use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    sync::mpsc,
    time::Duration,
};

#[derive(Clone, Default)]
pub struct Credentials {
    pub login: String,
    pub sudo: String,
    pub same_password: bool,
}
impl Credentials {
    pub fn sudo_password(&self) -> &str {
        if self.same_password {
            &self.login
        } else {
            &self.sudo
        }
    }
}
#[derive(Serialize, Deserialize)]
struct Request {
    prompt: String,
    kind: String,
    #[cfg(windows)]
    token: String,
}
#[derive(Serialize, Deserialize)]
struct Response {
    answer: Option<String>,
}

fn host_confirmation(request: &Request) -> bool {
    // OpenSSH's host-key confirmation uses RP_ECHO, not RP_ASK_PERMISSION,
    // so SSH_ASKPASS_PROMPT is normally unset for this question.
    let prompt = request.prompt.trim_end();
    prompt.contains("fingerprint")
        && (prompt
            .ends_with("Are you sure you want to continue connecting (yes/no/[fingerprint])?")
            || prompt.ends_with("Are you sure you want to continue connecting (yes/no)?"))
}

// Called only by OpenSSH's askpass hook; no password is placed in argv or env.
pub fn askpass() -> Result<(), String> {
    let path = std::env::var_os("CODESYNC_AUTH_SOCKET").ok_or("Missing authentication socket")?;
    #[cfg(windows)]
    let path = path.to_string_lossy().into_owned();
    let mut stream = LocalStream::connect(path).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(180)))
        .map_err(|e| e.to_string())?;
    let request = Request {
        prompt: std::env::args().nth(1).unwrap_or_default(),
        kind: std::env::var("SSH_ASKPASS_PROMPT").unwrap_or_default(),
        #[cfg(windows)]
        token: std::env::var("CODESYNC_AUTH_TOKEN").map_err(|e| e.to_string())?,
    };
    serde_json::to_writer(&mut stream, &request).map_err(|e| e.to_string())?;
    stream.write_all(b"\n").map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    let response: Response = serde_json::from_str(&line).map_err(|e| e.to_string())?;
    match response.answer {
        Some(answer) => {
            println!("{answer}");
            Ok(())
        }
        None => Err("Authentication cancelled or credentials unavailable".into()),
    }
}

pub struct Bridge {
    #[cfg(unix)]
    dir: PathBuf,
    #[cfg(windows)]
    pub token: String,
    pub socket: PathBuf,
    listener: LocalListener,
    pending: Option<(LocalStream, mpsc::Receiver<bool>)>,
}
impl Bridge {
    #[cfg(unix)]
    pub fn new() -> Result<Self, String> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "codesync-auth-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        codesync::platform::private_directory(&dir, false)
            .map_err(|e| format!("Cannot create private authentication directory: {e}"))?;
        let socket = dir.join("socket");
        let listener = match LocalListener::bind(&socket) {
            Ok(listener) => listener,
            Err(e) => {
                let _ = fs::remove_dir(&dir);
                return Err(e.to_string());
            }
        };
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        Ok(Self {
            dir,
            socket,
            listener,
            pending: None,
        })
    }
    #[cfg(windows)]
    pub fn new() -> Result<Self, String> {
        let mut random = [0u8; 32];
        getrandom::fill(&mut random).map_err(|e| e.to_string())?;
        let token = random.iter().map(|b| format!("{b:02x}")).collect();
        let listener = LocalListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let socket = PathBuf::from(
            listener
                .local_addr()
                .map_err(|e| e.to_string())?
                .to_string(),
        );
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        Ok(Self {
            token,
            socket,
            listener,
            pending: None,
        })
    }
    pub fn poll(
        &mut self,
        credentials: &Credentials,
        events: &mpsc::Sender<super::jobs::Event>,
        ctx: &eframe::egui::Context,
    ) {
        if let Some((_, response)) = &self.pending {
            match response.try_recv() {
                Ok(approved) => {
                    let (stream, _) = self.pending.take().unwrap();
                    reply(stream, approved.then(|| "yes".into()));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    let (stream, _) = self.pending.take().unwrap();
                    reply(stream, None);
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
            return;
        }
        let Ok((stream, _)) = self.listener.accept() else {
            return;
        };
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        if (&mut reader).take(65_536).read_line(&mut line).is_err() || !line.ends_with('\n') {
            return;
        }
        let Ok(request) = serde_json::from_str::<Request>(&line) else {
            return;
        };
        #[cfg(windows)]
        if request.token != self.token {
            return;
        }
        let stream = reader.into_inner();
        if host_confirmation(&request) {
            let (sender, response) = mpsc::channel();
            let _ = events.send(super::jobs::Event::TrustHost {
                prompt: request.prompt,
                response: sender,
            });
            self.pending = Some((stream, response));
            ctx.request_repaint();
        } else if request.kind.is_empty()
            && request
                .prompt
                .trim_end()
                .to_lowercase()
                .ends_with("password:")
            && !credentials.login.is_empty()
        {
            reply(stream, Some(credentials.login.clone()));
        } else {
            reply(stream, None);
            let _ = events.send(super::jobs::Event::Output(b"Authentication needs attention. Edit the server's login password, or unlock your SSH key in ssh-agent.\n".to_vec()));
            ctx.request_repaint();
        }
    }
}
fn reply(mut stream: LocalStream, answer: Option<String>) {
    let _ = serde_json::to_writer(&mut stream, &Response { answer });
    let _ = stream.write_all(b"\n");
}
impl Drop for Bridge {
    fn drop(&mut self) {
        if let Some((stream, _)) = self.pending.take() {
            reply(stream, None);
        }
        #[cfg(unix)]
        {
            let _ = fs::remove_file(&self.socket);
            let _ = fs::remove_dir(&self.dir);
        }
    }
}
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn credentials_use_private_ipc_and_host_trust_is_explicit() {
        let mut bridge = Bridge::new().unwrap();
        assert_eq!(
            fs::metadata(&bridge.dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let (events, receiver) = mpsc::channel();
        let ctx = eframe::egui::Context::default();
        let creds = Credentials {
            login: "secret-test".into(),
            ..Default::default()
        };
        let mut socket = LocalStream::connect(&bridge.socket).unwrap();
        writeln!(
            socket,
            "{{\"prompt\":\"user@host's password: \",\"kind\":\"\"}}"
        )
        .unwrap();
        bridge.poll(&creds, &events, &ctx);
        let mut line = String::new();
        BufReader::new(socket).read_line(&mut line).unwrap();
        assert_eq!(
            serde_json::from_str::<Response>(&line)
                .unwrap()
                .answer
                .as_deref(),
            Some("secret-test")
        );
        assert!(receiver.try_recv().is_err());
        for (question, kind, approved) in [
            (
                "Are you sure you want to continue connecting (yes/no/[fingerprint])? ",
                "",
                true,
            ),
            (
                "Are you sure you want to continue connecting (yes/no/[fingerprint])? ",
                "",
                false,
            ),
            (
                "Are you sure you want to continue connecting (yes/no)? ",
                "",
                true,
            ),
        ] {
            let mut socket = LocalStream::connect(&bridge.socket).unwrap();
            let request = Request {
                prompt: format!(
                    "The authenticity of host 'codesync-server-test' can't be established.\nED25519 key fingerprint is SHA256:test.\n{question}"
                ),
                kind: kind.into(),
            };
            serde_json::to_writer(&mut socket, &request).unwrap();
            writeln!(socket).unwrap();
            bridge.poll(&creds, &events, &ctx);
            if let super::super::jobs::Event::TrustHost { response, .. } =
                receiver.recv_timeout(Duration::from_secs(1)).unwrap()
            {
                response.send(approved).unwrap();
            } else {
                panic!("expected trust dialog, not password warning");
            }
            bridge.poll(&creds, &events, &ctx);
            let mut line = String::new();
            BufReader::new(socket).read_line(&mut line).unwrap();
            assert_eq!(
                serde_json::from_str::<Response>(&line)
                    .unwrap()
                    .answer
                    .as_deref(),
                approved.then_some("yes")
            );
        }
    }

    #[test]
    fn other_prompts_are_not_host_confirmations() {
        for (prompt, kind) in [
            ("user@host's password: ", ""),
            ("Enter passphrase for key '/tmp/key': ", ""),
            ("Allow access to a private key?", "confirm"),
            ("Please type 'yes', 'no' or the fingerprint: ", ""),
        ] {
            assert!(!host_confirmation(&Request {
                prompt: prompt.into(),
                kind: kind.into()
            }));
        }
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    #[test]
    fn loopback_auth_requires_token_and_explicit_host_trust() {
        let mut bridge = Bridge::new().unwrap();
        let (events, receiver) = mpsc::channel();
        let ctx = eframe::egui::Context::default();
        let credentials = Credentials {
            login: "test-secret".into(),
            ..Default::default()
        };
        for valid in [false, true] {
            let mut stream = LocalStream::connect(bridge.socket.to_str().unwrap()).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let request = Request {
                prompt: "user@host's password: ".into(),
                kind: String::new(),
                token: if valid {
                    bridge.token.clone()
                } else {
                    "wrong-token".into()
                },
            };
            serde_json::to_writer(&mut stream, &request).unwrap();
            writeln!(stream).unwrap();
            bridge.poll(&credentials, &events, &ctx);
            let mut line = String::new();
            let result = BufReader::new(stream).read_line(&mut line);
            if valid {
                assert_eq!(
                    serde_json::from_str::<Response>(&line)
                        .unwrap()
                        .answer
                        .as_deref(),
                    Some("test-secret")
                );
            } else {
                assert!(result.is_err() || line.is_empty());
            }
        }
        let mut stream = LocalStream::connect(bridge.socket.to_str().unwrap()).unwrap();
        let request = Request { prompt: "ED25519 key fingerprint is SHA256:test.\nAre you sure you want to continue connecting (yes/no/[fingerprint])?".into(), kind: String::new(), token: bridge.token.clone() };
        serde_json::to_writer(&mut stream, &request).unwrap();
        writeln!(stream).unwrap();
        bridge.poll(&credentials, &events, &ctx);
        let super::super::jobs::Event::TrustHost { response, .. } =
            receiver.recv_timeout(Duration::from_secs(2)).unwrap()
        else {
            panic!("Expected explicit trust prompt");
        };
        response.send(false).unwrap();
        bridge.poll(&credentials, &events, &ctx);
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        assert!(
            serde_json::from_str::<Response>(&line)
                .unwrap()
                .answer
                .is_none()
        );
    }
}
