use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{
        fs::DirBuilderExt,
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
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
    let mut stream = UnixStream::connect(path).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(180)))
        .map_err(|e| e.to_string())?;
    let request = Request {
        prompt: std::env::args().nth(1).unwrap_or_default(),
        kind: std::env::var("SSH_ASKPASS_PROMPT").unwrap_or_default(),
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
    dir: PathBuf,
    pub socket: PathBuf,
    listener: UnixListener,
    pending: Option<(UnixStream, mpsc::Receiver<bool>)>,
}
impl Bridge {
    pub fn new() -> Result<Self, String> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "codesync-auth-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&dir)
            .map_err(|e| format!("Cannot create private authentication directory: {e}"))?;
        let socket = dir.join("socket");
        let listener = match UnixListener::bind(&socket) {
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
        let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() {
            return;
        }
        let Ok(request) = serde_json::from_str::<Request>(&line) else {
            return;
        };
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
fn reply(mut stream: UnixStream, answer: Option<String>) {
    let _ = serde_json::to_writer(&mut stream, &Response { answer });
    let _ = stream.write_all(b"\n");
}
impl Drop for Bridge {
    fn drop(&mut self) {
        if let Some((stream, _)) = self.pending.take() {
            reply(stream, None);
        }
        let _ = fs::remove_file(&self.socket);
        let _ = fs::remove_dir(&self.dir);
    }
}
#[cfg(test)]
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
        let mut socket = UnixStream::connect(&bridge.socket).unwrap();
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
            let mut socket = UnixStream::connect(&bridge.socket).unwrap();
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
