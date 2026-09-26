use super::{
    auth::{Bridge, Credentials},
    store::{ConnectionMode, Server},
};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::{
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};

pub enum Event {
    BrowserListing {
        server: u64,
        listing: super::browser::Listing,
    },
    BrowserPreview {
        server: u64,
        preview: super::browser::Preview,
    },
    Output(Vec<u8>),
    TrustHost {
        prompt: String,
        response: Sender<bool>,
    },
    Login {
        label: String,
        url: String,
    },
    #[cfg(unix)]
    HostPassword {
        purpose: String,
        response: Sender<Option<String>>,
    },
    TailscaleReady {
        server: u64,
        host: String,
        port: u16,
    },
    Finished(Result<(), String>),
}
#[derive(Clone)]
pub enum Action {
    Browse { path: String, preview: bool },
    Test,
    Setup,
    Tailscale,
    PrepareHost,
    DisableHost,
    Sync,
}
#[derive(Clone)]
pub struct Task {
    pub server: Server,
    pub credentials: Credentials,
    pub local: PathBuf,
    pub remote: String,
    pub action: Action,
}
pub struct Job {
    pub events: Receiver<Event>,
    cancel: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Job {
    pub fn start(tasks: Vec<Task>, ctx: eframe::egui::Context) -> Self {
        let (events_tx, events) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancelled = cancel.clone();
        let worker = thread::spawn(move || {
            let result = run_tasks(tasks, &cancelled, &events_tx, &ctx);
            let _ = events_tx.send(Event::Finished(result));
            ctx.request_repaint();
        });
        Self {
            events,
            cancel,
            worker: Some(worker),
        }
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(unix)]
pub fn host_ssh_status() -> Result<bool, String> {
    let output = Command::new("sh")
        .args([
            "-c",
            include_str!("host-control.sh"),
            "codesync-host-status",
            "status",
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(
            "Cannot read SSH status. A running systemd or OpenRC service manager is required."
                .into(),
        );
    }
    match String::from_utf8_lossy(&output.stdout).trim() {
        "enabled" => Ok(true),
        "disabled" => Ok(false),
        _ => Err("Unexpected SSH service status.".into()),
    }
}
#[cfg(windows)]
pub fn host_ssh_status() -> Result<bool, String> {
    Err("Windows hosts connect to Linux servers; incoming SSH setup is Linux-only.".into())
}
fn ssh_args(server: &Server) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        &server.port.to_string(),
        "-o",
        "ControlMaster=auto",
        "-o",
        "ControlPersist=10m",
        "-o",
        &format!("ControlPath=~/.ssh/codesync-server-{}-%C", server.id),
        "-o",
        "NumberOfPasswordPrompts=1",
        "-o",
        "ConnectTimeout=15",
        "-o",
        "ServerAliveInterval=15",
        "-o",
        "ServerAliveCountMax=2",
        "-o",
        "StrictHostKeyChecking=yes",
        "-o",
        "PreferredAuthentications=publickey,password",
        "-o",
        "KbdInteractiveAuthentication=no",
        "-T",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    args.extend([
        "-o".into(),
        format!("HostKeyAlias={}", server.identity_alias()),
        "-o".into(),
        "UserKnownHostsFile=~/.ssh/codesync_known_hosts".into(),
        "-o".into(),
        "GlobalKnownHostsFile=/dev/null".into(),
        "-o".into(),
        "CheckHostIP=no".into(),
    ]);
    if cfg!(windows) {
        if let Some(home) = codesync::platform::home() {
            for arg in &mut args {
                if arg.starts_with("UserKnownHostsFile=") {
                    *arg = format!(
                        "UserKnownHostsFile=\"{}\"",
                        codesync::platform::local_path(&home.join(".ssh/codesync_known_hosts"))
                    );
                }
            }
        }
        for arg in &mut args {
            if arg == "ControlMaster=auto" {
                *arg = "ControlMaster=no".into();
            }
            if arg.starts_with("ControlPath=") {
                *arg = "ControlPath=none".into();
            }
        }
    }
    args
}
fn ssh(server: &Server, script: &str) -> Command {
    let mut command = codesync::platform::command("ssh");
    command
        .args(ssh_args(server))
        .arg(server.destination())
        .arg(script);
    command
}
fn sync_command(task: &Task, pull: bool, dry: bool) -> Command {
    let mut cmd = codesync::platform::command("rsync");
    let transport = std::iter::once("ssh".to_owned())
        .chain(ssh_args(&task.server))
        .map(|s| codesync::quote(&s))
        .collect::<Vec<_>>()
        .join(" ");
    cmd.args(["-az", "--protect-args", "--itemize-changes", "-e"])
        .arg(transport);
    for pattern in codesync::EXCLUDES {
        cmd.arg(format!("--exclude={pattern}"));
    }
    if dry {
        cmd.arg("--dry-run");
    }
    let remote = codesync::rsync_destination(&task.server.destination(), &task.remote);
    codesync::platform::rsync_options(&mut cmd);
    cmd.arg("--");
    if pull {
        cmd.arg(remote).arg("./");
    } else {
        cmd.arg("./").arg(remote);
    }
    cmd
}
fn setup_script() -> String {
    let install = format!("sh -c {}", codesync::quote(include_str!("../setup.sh")));
    format!(
        "if command -v rsync >/dev/null 2>&1; then rsync --version; elif [ \"$(id -u)\" = 0 ]; then {install}; elif command -v sudo >/dev/null 2>&1; then sudo -S -p '' -- {install}; elif command -v doas >/dev/null 2>&1; then doas -n {install}; else printf '%s\\n' 'Install rsync manually, or enable sudo for this account.' >&2; exit 1; fi"
    )
}
fn run_tasks(
    tasks: Vec<Task>,
    cancelled: &AtomicBool,
    events: &Sender<Event>,
    ctx: &eframe::egui::Context,
) -> Result<(), String> {
    let mut bridge = Bridge::new()?;
    let mut sync_tasks = Vec::new();
    for task in tasks {
        if cancelled.load(Ordering::Relaxed) {
            return Err("Stopped. Completed transfers remain in place.".into());
        }
        if matches!(task.action, Action::PrepareHost | Action::DisableHost) {
            let mut runner = Runner {
                task: &task,
                bridge: &mut bridge,
                cancelled,
                events,
                ctx,
            };
            if matches!(task.action, Action::DisableHost) {
                runner.disable_host()?;
            } else {
                runner.setup_host(
                    &local_host_setup_script(),
                    "incoming SSH connections",
                    OutputMode::Live,
                )?;
            }
            continue;
        }
        task.server.validate()?;
        let task = resolve_endpoint(task, &mut bridge, cancelled, events, ctx)?;
        let _ = events.send(Event::Output(
            format!("\n--- {} ---\n", task.server.name).into_bytes(),
        ));
        let mut runner = Runner {
            task: &task,
            bridge: &mut bridge,
            cancelled,
            events,
            ctx,
        };
        match &task.action {
            Action::Browse { path, preview } => {
                let script = super::browser::remote_script(path, *preview)?;
                let output =
                    runner.execute_output(ssh(&task.server, &script), None, OutputMode::Capture)?;
                let event = if *preview {
                    Event::BrowserPreview {
                        server: task.server.id,
                        preview: super::browser::remote_preview(path, output),
                    }
                } else {
                    Event::BrowserListing {
                        server: task.server.id,
                        listing: super::browser::remote_listing(&output)?,
                    }
                };
                let _ = events.send(event);
                ctx.request_repaint();
            }
            Action::Test => runner.execute(
                ssh(&task.server, "printf '%s\\n' 'Connected successfully.'"),
                None,
            )?,
            Action::Setup => {
                runner.execute(
                    ssh(&task.server, &setup_script()),
                    Some(task.credentials.sudo_password()),
                )?;
            }
            Action::Tailscale => runner.setup_tailscale()?,
            Action::PrepareHost | Action::DisableHost => unreachable!(),
            Action::Sync => {
                sync_tasks.push(task.clone());
                continue;
            }
        }
        let _ = events.send(Event::Output(b"Completed.\n".to_vec()));
        ctx.request_repaint();
    }
    sync_groups(sync_tasks, &mut bridge, cancelled, events, ctx)?;
    Ok(())
}
fn copy_command(
    source: &std::path::Path,
    destination: &std::path::Path,
    missing_only: bool,
) -> Command {
    let mut cmd = codesync::platform::command("rsync");
    cmd.args(["-a", "--itemize-changes", "--omit-dir-times"]);
    for pattern in codesync::EXCLUDES {
        cmd.arg(format!("--exclude={pattern}"));
    }
    if missing_only {
        cmd.arg("--ignore-existing");
    }
    codesync::platform::rsync_options(&mut cmd);
    cmd.arg("--")
        .arg(format!("{}/", codesync::platform::local_path(source)))
        .arg(codesync::platform::local_path(destination));
    cmd
}
fn verify_command(
    source: &std::path::Path,
    destination: &std::ffi::OsStr,
    server: Option<&Server>,
) -> Command {
    let mut cmd = codesync::platform::command("rsync");
    cmd.args(["-rcn", "--out-format=%n", "--protect-args"]);
    for pattern in codesync::EXCLUDES {
        cmd.arg(format!("--exclude={pattern}"));
    }
    if let Some(server) = server {
        let transport = std::iter::once("ssh".to_owned())
            .chain(ssh_args(server))
            .map(|s| codesync::quote(&s))
            .collect::<Vec<_>>()
            .join(" ");
        cmd.arg("-e").arg(transport);
    }
    codesync::platform::rsync_options(&mut cmd);
    cmd.arg("--")
        .arg(format!("{}/", codesync::platform::local_path(source)))
        .arg(if server.is_some() {
            destination.to_string_lossy().into_owned()
        } else {
            codesync::platform::local_path(std::path::Path::new(destination))
        });
    cmd
}
fn remote_rename_script(root: &str, rename: &super::sync::Rename) -> Result<String, String> {
    let path = |p: &std::path::Path| {
        p.to_str()
            .map(|s| {
                codesync::quote(&if cfg!(windows) {
                    s.replace('\\', "/")
                } else {
                    s.into()
                })
            })
            .ok_or_else(|| "Remote filenames must be valid UTF-8 for conflict renaming.".to_owned())
    };
    let from = path(&rename.from)?;
    let to = path(&rename.to)?;
    let mut script = format!("set -eu; cd -- {}; ", codesync::quote(root));
    for ancestor in rename
        .from
        .ancestors()
        .filter(|p| !p.as_os_str().is_empty())
    {
        script.push_str(&format!("test ! -L {} || exit 1; ", path(ancestor)?));
    }
    script.push_str(&format!("test -f {from}; test ! -e {to}; test ! -L {to}; actual=$(sha256sum < {from}); test \"${{actual%% *}}\" = {} || {{ echo 'File changed during sync; retry after edits finish.' >&2; exit 1; }}; mv -nT -- {from} {to}; test ! -e {from}; test ! -L {from}", codesync::quote(&rename.hash)));
    Ok(script)
}
fn sync_groups(
    tasks: Vec<Task>,
    bridge: &mut Bridge,
    cancelled: &AtomicBool,
    events: &Sender<Event>,
    ctx: &eframe::egui::Context,
) -> Result<(), String> {
    use super::sync;
    let mut groups: std::collections::BTreeMap<PathBuf, Vec<Task>> =
        std::collections::BTreeMap::new();
    for task in tasks {
        groups.entry(task.local.clone()).or_default().push(task);
    }
    for (local, tasks) in groups {
        sync::check_cancel(cancelled)?;
        let scratch = sync::Scratch::new()?;
        let mut snapshots = vec![scratch.0.join("local")];
        std::fs::create_dir(&snapshots[0]).map_err(|e| e.to_string())?;
        let mut runner = Runner {
            task: &tasks[0],
            bridge,
            cancelled,
            events,
            ctx,
        };
        let _ = events.send(Event::Output(
            b"Reading copies and comparing SHA-256 hashes...\n".to_vec(),
        ));
        if cfg!(windows) {
            let mut inspect = codesync::platform::command("sh");
            inspect.args(["-c", &codesync::platform::windows_manifest_script(".")]);
            let manifest = runner.execute_output(inspect, None, OutputMode::Capture)?;
            codesync::platform::validate_windows_manifest(&manifest)?;
        }
        runner.execute(copy_command(&local, &snapshots[0], false), None)?;
        let host_name = std::env::var("COMPUTERNAME")
            .or_else(|_| std::fs::read_to_string("/etc/hostname"))
            .unwrap_or_else(|_| "Host".into())
            .trim()
            .to_owned();
        let mut names = vec![host_name];
        for (index, task) in tasks.iter().enumerate() {
            codesync::Config {
                host: task.server.destination(),
                dir: task.remote.clone(),
            }
            .validate()?;
            let mut runner = Runner {
                task,
                bridge,
                cancelled,
                events,
                ctx,
            };
            runner.execute(ssh(&task.server, &format!("command -v sha256sum >/dev/null || {{ echo 'Install sha256sum on this host before syncing.' >&2; exit 1; }}; mkdir -p -- {}", codesync::quote(&task.remote))), None)?;
            if cfg!(windows) {
                let manifest = runner.execute_output(
                    ssh(
                        &task.server,
                        &codesync::platform::windows_manifest_script(&task.remote),
                    ),
                    None,
                    OutputMode::Capture,
                )?;
                codesync::platform::validate_windows_manifest(&manifest)?;
            }
            let directory = scratch.0.join(format!("remote-{index}"));
            std::fs::create_dir(&directory).map_err(|e| e.to_string())?;
            let mut snapshot_task = task.clone();
            snapshot_task.local = directory.clone();
            Runner {
                task: &snapshot_task,
                bridge,
                cancelled,
                events,
                ctx,
            }
            .execute(sync_command(&snapshot_task, true, false), None)?;
            snapshots.push(directory);
            names.push(task.server.name.clone());
        }
        let trees = snapshots
            .iter()
            .map(|path| sync::scan(path, cancelled))
            .collect::<Result<Vec<_>, _>>()?;
        let plan = sync::plan(&trees, &names)?;
        if cfg!(windows) {
            let paths: Vec<_> = trees
                .iter()
                .flat_map(|tree| tree.files.keys().chain(tree.dirs.iter()))
                .chain(plan.files.keys())
                .chain(plan.dirs.iter())
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .collect();
            codesync::platform::validate_windows_paths(paths.iter().map(String::as_str))?;
        }
        let merged = scratch.0.join("merged");
        sync::materialize(&plan, &snapshots, &merged, cancelled)?;
        let _ = events.send(Event::Output(
            format!(
                "{} files to keep; {} conflict versions to rename.\n",
                plan.files.len(),
                plan.renames.len()
            )
            .into_bytes(),
        ));
        // Validate every remote script before changing any originals.
        let scripts = plan
            .renames
            .iter()
            .filter(|r| r.participant > 0)
            .map(|r| {
                remote_rename_script(&tasks[r.participant - 1].remote, r)
                    .map(|script| (r.participant - 1, script))
            })
            .collect::<Result<Vec<_>, _>>()?;
        for (index, script) in scripts {
            let task = &tasks[index];
            Runner {
                task,
                bridge,
                cancelled,
                events,
                ctx,
            }
            .execute(ssh(&task.server, &script), None)?;
        }
        for rename in plan.renames.iter().filter(|r| r.participant == 0) {
            sync::check_cancel(cancelled)?;
            sync::local_rename(&local, rename)?;
        }
        let mut runner = Runner {
            task: &tasks[0],
            bridge,
            cancelled,
            events,
            ctx,
        };
        runner.execute(copy_command(&merged, &local, true), None)?;
        for task in &tasks {
            let mut source = task.clone();
            source.local = merged.clone();
            let mut cmd = sync_command(&source, false, false);
            // Insert before -- and positional arguments.
            let args: Vec<_> = cmd.get_args().map(|arg| arg.to_os_string()).collect();
            cmd = codesync::platform::command("rsync");
            cmd.arg("--ignore-existing")
                .arg("--omit-dir-times")
                .args(args)
                .current_dir(&merged);
            Runner {
                task: &source,
                bridge,
                cancelled,
                events,
                ctx,
            }
            .execute(cmd, None)?;
        }
        let mut runner = Runner {
            task: &tasks[0],
            bridge,
            cancelled,
            events,
            ctx,
        };
        if !runner
            .execute_output(
                verify_command(&merged, local.as_os_str(), None),
                None,
                OutputMode::Capture,
            )?
            .trim()
            .is_empty()
        {
            return Err("Local files changed during sync. Preserved all versions; run Sync again after edits finish.".into());
        }
        for task in &tasks {
            let remote = codesync::rsync_destination(&task.server.destination(), &task.remote);
            let mut runner = Runner {
                task,
                bridge,
                cancelled,
                events,
                ctx,
            };
            if !runner
                .execute_output(
                    verify_command(&merged, std::ffi::OsStr::new(&remote), Some(&task.server)),
                    None,
                    OutputMode::Capture,
                )?
                .trim()
                .is_empty()
            {
                return Err(format!(
                    "Files on {} changed during sync. Run Sync again after edits finish.",
                    task.server.name
                ));
            }
        }
        let _ = events.send(Event::Output(
            b"Folder sync completed. No deletions propagated.\n".to_vec(),
        ));
    }
    Ok(())
}
struct Runner<'a> {
    task: &'a Task,
    bridge: &'a mut Bridge,
    cancelled: &'a AtomicBool,
    events: &'a Sender<Event>,
    ctx: &'a eframe::egui::Context,
}
impl Runner<'_> {
    fn execute(&mut self, command: Command, input: Option<&str>) -> Result<(), String> {
        self.execute_output(command, input, OutputMode::Live)
            .map(|_| ())
    }
    fn execute_output(
        &mut self,
        mut command: Command,
        input: Option<&str>,
        mode: OutputMode,
    ) -> Result<String, String> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err("Stopped.".into());
        }
        command
            .current_dir(&self.task.local)
            .env(
                "SSH_ASKPASS",
                codesync::platform::local_path(
                    &std::env::current_exe().map_err(|e| e.to_string())?,
                ),
            )
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("CODESYNC_ASKPASS", "1")
            .env("CODESYNC_AUTH_SOCKET", &self.bridge.socket)
            .env("LC_ALL", "C")
            .env("TERM", "dumb")
            .env("NO_COLOR", "1")
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        command.process_group(0);
        #[cfg(windows)]
        command.env("CODESYNC_AUTH_TOKEN", &self.bridge.token);
        let mut child = command.spawn().map_err(|e| {
            format!(
                "Cannot start {}: {e}",
                command.get_program().to_string_lossy()
            )
        })?;
        if let Some(secret) = input
            && let Some(mut stdin) = child.stdin.take()
        {
            // Only the fixed setup command gets the sudo password, through stdin.
            let _ = stdin.write_all(secret.as_bytes());
            let _ = stdin.write_all(b"\n");
        }
        let mut readers = Vec::new();
        for mut pipe in [
            Box::new(child.stdout.take().unwrap()) as Box<dyn Read + Send>,
            Box::new(child.stderr.take().unwrap()),
        ] {
            let events = self.events.clone();
            let ctx = self.ctx.clone();
            readers.push(thread::spawn(move || {
                let mut bytes = [0u8; 4096];
                let mut captured = Vec::new();
                let mut pending = Vec::new();
                while let Ok(count) = pipe.read(&mut bytes) {
                    if count == 0 {
                        break;
                    }
                    if captured.len() < 2_000_000 {
                        captured.extend_from_slice(&bytes[..count]);
                    }
                    match mode {
                        OutputMode::Live => {
                            let _ = events.send(Event::Output(bytes[..count].to_vec()));
                        }
                        OutputMode::Capture => {}
                        OutputMode::Tailscale(label) => {
                            pending.extend_from_slice(&bytes[..count]);
                            while let Some(end) = pending.iter().position(|b| *b == b'\n') {
                                let line: Vec<_> = pending.drain(..=end).collect();
                                emit_tailscale_line(&line, label, &events);
                            }
                            if pending.len() > 16_384 {
                                emit_tailscale_line(&pending, label, &events);
                                pending.clear();
                            }
                        }
                    }
                    ctx.request_repaint();
                }
                if let OutputMode::Tailscale(label) = mode
                    && !pending.is_empty()
                {
                    emit_tailscale_line(&pending, label, &events);
                    ctx.request_repaint();
                }
                captured
            }));
        }
        let mut stopping = None;
        let status = loop {
            if self.cancelled.load(Ordering::Relaxed) && stopping.is_none() {
                stop_child(&mut child, false);
                stopping = Some(Instant::now());
            }
            self.bridge
                .poll(&self.task.credentials, self.events, self.ctx);
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                break status;
            }
            if stopping.is_some_and(|time| time.elapsed() > Duration::from_secs(2)) {
                stop_child(&mut child, true);
            }
            thread::sleep(Duration::from_millis(30));
        };
        let mut streams = readers
            .into_iter()
            .map(|reader| reader.join().unwrap_or_default());
        let stdout = streams.next().unwrap_or_default();
        let stderr = streams.next().unwrap_or_default();
        for _ in streams {}

        if stopping.is_some() {
            Err("Stopped. Files already transferred remain in place.".into())
        } else if status.success() {
            Ok(String::from_utf8_lossy(&stdout).into_owned())
        } else {
            let program = std::path::Path::new(command.get_program())
                .file_name()
                .unwrap_or(command.get_program())
                .to_string_lossy();
            Err(format!(
                "{program} failed ({status}). {}",
                failure_reason(&String::from_utf8_lossy(&stderr))
            ))
        }
    }
}
#[derive(Clone, Copy)]
enum OutputMode {
    Live,
    Capture,
    Tailscale(&'static str),
}

fn failure_reason(stderr: &str) -> &'static str {
    let error = stderr.to_lowercase();
    if error.contains("identification has changed")
        || error.contains("host key verification failed")
    {
        "The SSH server identity was not accepted. Verify the fingerprint; a different machine may be using this address."
    } else if error.contains("could not resolve hostname") {
        "The server hostname could not be resolved. Check its address and DNS."
    } else if error.contains("connection refused") {
        "The SSH port refused the connection. Check the port and whether SSH is running."
    } else if error.contains("timed out")
        || error.contains("no route to host")
        || error.contains("network is unreachable")
    {
        "The server could not be reached. Check the selected connection mode and network."
    } else if error.contains("incorrect password") || error.contains("sorry, try again") {
        "The administrator password was rejected."
    } else if error.contains("a password is required") || error.contains("no password was provided")
    {
        "Administrator authentication is required."
    } else if error.contains("permission denied") || error.contains("authentication failed") {
        "Authentication was rejected. Check the credentials for this step."
    } else if error.contains("authentication agent") || error.contains("not authorized") {
        "Desktop administrator authorization was unavailable or cancelled."
    } else if error.contains("bad owner or permissions") {
        "SSH refused a configuration file because of its ownership or permissions."
    } else {
        "Check the activity output for this step."
    }
}

#[cfg(unix)]
fn wait_for_host_password(
    receiver: Receiver<Option<String>>,
    cancelled: &AtomicBool,
) -> Result<String, String> {
    let started = Instant::now();
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return Err("Stopped.".into());
        }
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(Some(password)) => return Ok(password),
            Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("Host authorization cancelled.".into());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if started.elapsed() > Duration::from_secs(300) {
            return Err("Host authorization timed out. Retry setup.".into());
        }
    }
}
fn available(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| {
            dir.join(format!("{program}{}", std::env::consts::EXE_SUFFIX))
                .is_file()
        })
    })
}
fn tailscale_running(output: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(output)
        .is_ok_and(|status| status["BackendState"].as_str() == Some("Running"))
}

fn emit_tailscale_line(bytes: &[u8], label: &str, events: &Sender<Event>) {
    if let Some(url) = login_url(&String::from_utf8_lossy(bytes)) {
        let _ = events.send(Event::Login {
            label: label.into(),
            url,
        });
        let _ = events.send(Event::Output(
            format!("{label}: sign-in required. Use the sign-in button above.\n").into_bytes(),
        ));
    } else {
        let _ = events.send(Event::Output(bytes.to_vec()));
    }
}
fn login_url(line: &str) -> Option<String> {
    line.split_whitespace().find_map(|word| {
        let url = word.trim_matches(['"', '\'', '(', ')', '<', '>']);
        let path = url.strip_prefix("https://login.tailscale.com/")?;
        (!path.is_empty()
            && path
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "/?=&-_.%".contains(c)))
        .then(|| url.into())
    })
}
fn identity_known(server: &Server) -> Result<bool, String> {
    let home = codesync::platform::home().ok_or("Cannot locate SSH identity storage.")?;
    let dir = home.join(".ssh");
    codesync::platform::private_directory(&dir, true).map_err(|e| e.to_string())?;
    let file = dir.join("codesync_known_hosts");
    if !file.exists() {
        return Ok(false);
    }
    let output = codesync::platform::command("ssh-keygen")
        .arg("-F")
        .arg(server.identity_alias())
        .arg("-f")
        .arg(codesync::platform::local_path(&file))
        .output()
        .map_err(|e| format!("Cannot inspect saved server identity: {e}"))?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err("Cannot read the saved SSH server identities.".into()),
    }
}
fn peer_address(json: &str, configured: &str) -> Option<String> {
    let status: serde_json::Value = serde_json::from_str(json).ok()?;
    if status["BackendState"].as_str() != Some("Running") {
        return None;
    }
    let configured = configured
        .trim_matches(['[', ']'])
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let mut matches = Vec::new();
    for peer in status["Peer"].as_object()?.values() {
        let Some(peer_ips) = peer["TailscaleIPs"].as_array() else {
            continue;
        };
        let ips: Vec<_> = peer_ips
            .iter()
            .filter_map(|ip| ip.as_str())
            .filter_map(|ip| ip.parse::<std::net::IpAddr>().ok())
            .collect();
        let dns = peer["DNSName"]
            .as_str()
            .unwrap_or("")
            .trim_end_matches('.')
            .to_ascii_lowercase();
        let hostname = peer["HostName"].as_str().unwrap_or("").to_ascii_lowercase();
        let configured_ip = configured.parse::<std::net::IpAddr>().ok();
        let address_match = configured_ip.is_some_and(|ip| ips.contains(&ip));
        let name_match = configured_ip.is_none()
            && !configured.is_empty()
            && (configured == dns
                || configured == hostname
                || dns.split('.').next() == Some(configured.as_str()));
        if (address_match || name_match)
            && let Some(ip) = ips.iter().find(|ip| ip.is_ipv4()).or(ips.first())
        {
            matches.push(ip.to_string());
        }
    }
    // Hostnames may be duplicated: never guess which peer the user intended.
    if matches.len() == 1 {
        matches.pop()
    } else {
        None
    }
}
fn tailscale_membership(json: &str, address: &str) -> bool {
    peer_address(json, address).is_some()
}
fn resolve_endpoint(
    mut task: Task,
    bridge: &mut Bridge,
    cancelled: &AtomicBool,
    events: &Sender<Event>,
    ctx: &eframe::egui::Context,
) -> Result<Task, String> {
    let known = identity_known(&task.server)?;
    if !known && matches!(task.action, Action::Sync) {
        return Err("Confirm this server's identity first: select the server and use Test connection. Check its fingerprint against the intended server.".into());
    }
    let local_tailnet =
        if task.server.network.mode != ConnectionMode::LocalOnly && available("tailscale") {
            let mut command = Command::new("tailscale");
            command.args(["status", "--json"]);
            Runner {
                task: &task,
                bridge,
                cancelled,
                events,
                ctx,
            }
            .execute_output(command, None, OutputMode::Capture)
            .ok()
            .filter(|status| tailscale_running(status))
        } else {
            None
        };
    if task.server.network.tailscale_host.is_empty()
        && let Some(status) = &local_tailnet
        && let Some(address) = peer_address(status, &task.server.host)
    {
        if task
            .server
            .host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok()
        {
            task.server.network.tailscale_port = task.server.port;
        }
        task.server.network.tailscale_host = address;
    }
    let endpoints = task.server.endpoints();
    if endpoints.is_empty() {
        return Err("Remote only needs a public address or completed Tailscale setup. Edit the server's connection settings.".into());
    }
    for (host, port, label) in endpoints {
        if cancelled.load(Ordering::Relaxed) {
            return Err("Stopped.".into());
        }
        let mut candidate = task.clone();
        candidate.server.host = host.into();
        candidate.server.port = port;
        let _ = events.send(Event::Output(
            format!("Checking {label} connection against this server's SSH identity...\n")
                .into_bytes(),
        ));
        let mut args = ssh_args(&candidate.server);
        if !known {
            for arg in &mut args {
                if arg == "StrictHostKeyChecking=yes" {
                    *arg = "StrictHostKeyChecking=ask".into();
                }
                if arg == "ControlMaster=auto" {
                    *arg = "ControlMaster=no".into();
                }
                if arg.starts_with("ControlPath=") {
                    *arg = "ControlPath=none".into();
                }
            }
        }
        let mut command = codesync::platform::command("ssh");
        command
            .args(args)
            .arg(candidate.server.destination())
            .arg("true");
        let result = Runner {
            task: &candidate,
            bridge,
            cancelled,
            events,
            ctx,
        }
        .execute_output(command, None, OutputMode::Capture);
        if result.is_ok() {
            if !identity_known(&candidate.server)? {
                return Err("The server's SSH identity was not saved. Check ~/.ssh permissions and retry Test connection.".into());
            }
            if label == "Tailscale" {
                let _ = events.send(Event::TailscaleReady {
                    server: candidate.server.id,
                    host: candidate.server.host.clone(),
                    port: candidate.server.port,
                });
            } else if let Some(status) = &local_tailnet {
                // A LAN IP is not a peer identity. Ask the already-verified device
                // for its Tailscale address instead of guessing from its label.
                let mut runner = Runner {
                    task: &candidate,
                    bridge,
                    cancelled,
                    events,
                    ctx,
                };
                let remote_status = runner
                    .execute_output(
                        ssh(&candidate.server, "tailscale status --json"),
                        None,
                        OutputMode::Capture,
                    )
                    .ok();
                if let Some(host) = remote_status
                    .as_deref()
                    .and_then(|json| tailscale_address(json).ok())
                    .filter(|host| tailscale_membership(status, host))
                {
                    let port = runner
                        .execute_output(
                            ssh(&candidate.server, "printf '%s' \"${SSH_CONNECTION##* }\""),
                            None,
                            OutputMode::Capture,
                        )
                        .ok()
                        .and_then(|text| text.trim().parse::<u16>().ok())
                        .filter(|port| *port != 0);
                    if let Some(port) = port {
                        let mut preferred = candidate.clone();
                        preferred.server.host = host.clone();
                        preferred.server.port = port;
                        if runner
                            .execute_output(
                                ssh(&preferred.server, "true"),
                                None,
                                OutputMode::Capture,
                            )
                            .is_ok()
                        {
                            let _ = events.send(Event::TailscaleReady {
                                server: preferred.server.id,
                                host,
                                port,
                            });
                            let _ = events.send(Event::Output(
                                b"Using Tailscale; server identity verified.\n".to_vec(),
                            ));
                            return Ok(preferred);
                        }
                        let _ = events.send(Event::Output(b"Tailscale SSH is unavailable or its identity did not match; keeping the verified fallback connection.\n".to_vec()));
                    }
                }
            }
            if cancelled.load(Ordering::Relaxed) {
                return Err("Stopped.".into());
            }
            let _ = events.send(Event::Output(
                format!("Using {label}; server identity verified.\n").into_bytes(),
            ));
            return Ok(candidate);
        }
        if cancelled.load(Ordering::Relaxed) {
            return Err("Stopped.".into());
        }
        let reason = result.unwrap_err();
        if !known {
            return Err(format!(
                "Initial server connection failed: {reason} Select Local only or Remote only to choose where to confirm its fingerprint."
            ));
        }
        let _ = events.send(Event::Output(format!("{label}: {reason}\n").into_bytes()));
    }
    Err("No configured address connected with the saved SSH identity and credentials. No files were transferred. Check the server's addresses, credentials, and network access.".into())
}
#[cfg(unix)]
const STOP_SSH_HELPER: &str = "/usr/local/libexec/codesync-stop-ssh";
#[cfg(unix)]
fn stop_ssh_helper() -> String {
    format!(
        "#!/bin/sh\nPATH=/usr/sbin:/usr/bin:/sbin:/bin\nexport PATH\nunset ENV BASH_ENV CDPATH\n[ \"$#\" -eq 0 ] || exit 2\nset -- disable\n{}",
        include_str!("host-control.sh")
    )
}
#[cfg(unix)]
fn stop_ssh_policy(uid: u32) -> String {
    format!(
        "# Codesync: only stop SSH, never enable it or run arbitrary commands.\n#{uid} ALL=(root) NOPASSWD: NOSETENV: {STOP_SSH_HELPER} \"\"\n"
    )
}
#[cfg(unix)]
fn host_setup_script(uid: u32) -> String {
    let policy_setup = if uid == 0 {
        String::new()
    } else {
        format!(
            r#"
command -v sudo >/dev/null && command -v visudo >/dev/null || {{ echo 'Install sudo before preparing password-free SSH shutdown.' >&2; exit 1; }}
umask 077
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT HUP INT TERM
printf %s {helper} > "$work/helper"
printf %s {policy} > "$work/policy"
visudo -cf "$work/policy"
install -d -o 0 -g 0 -m 0755 /usr/local/libexec
install -o 0 -g 0 -m 0755 "$work/helper" {helper_path}
install -d -o 0 -g 0 -m 0750 /etc/sudoers.d
install -o 0 -g 0 -m 0440 "$work/policy" /etc/sudoers.d/codesync-stop-ssh-{uid}
rm -rf -- "$work"
trap - EXIT HUP INT TERM
"#,
            helper = codesync::quote(&stop_ssh_helper()),
            policy = codesync::quote(&stop_ssh_policy(uid)),
            helper_path = STOP_SSH_HELPER
        )
    };
    format!("set -eu\n{policy_setup}\n{}", include_str!("host-setup.sh"))
}
fn privileged_remote(script: &str) -> String {
    let command = format!("sh -c {}", codesync::quote(script));
    format!(
        "if [ \"$(id -u)\" = 0 ]; then {command}; elif command -v sudo >/dev/null 2>&1; then sudo -S -p '' -- {command}; elif command -v doas >/dev/null 2>&1; then doas -n {command}; else printf '%s\\n' 'Server setup requires root, sudo, or passwordless doas.' >&2; exit 1; fi"
    )
}
fn tailscale_address(json: &str) -> Result<String, String> {
    let status: serde_json::Value =
        serde_json::from_str(json).map_err(|_| "Cannot read Tailscale status.")?;
    if status["BackendState"].as_str() != Some("Running") {
        return Err("Tailscale is not connected. Finish signing in and retry setup.".into());
    }
    status["Self"]["TailscaleIPs"]
        .as_array()
        .and_then(|ips| {
            ips.iter()
                .filter_map(|v| v.as_str())
                .find(|s| s.parse::<std::net::Ipv4Addr>().is_ok())
                .or_else(|| {
                    ips.iter()
                        .filter_map(|v| v.as_str())
                        .find(|s| s.parse::<std::net::Ipv6Addr>().is_ok())
                })
        })
        .map(str::to_owned)
        .ok_or_else(|| "Tailscale did not return a server address.".into())
}
impl Runner<'_> {
    #[cfg(unix)]
    fn disable_host(&mut self) -> Result<(), String> {
        let mut command = if unsafe { libc::geteuid() } == 0 {
            let mut command = Command::new("sh");
            command.args([
                "-c",
                &format!("set -- disable\n{}", include_str!("host-control.sh")),
            ]);
            command
        } else {
            let mut command = Command::new("sudo");
            command.args(["-n", "--", STOP_SSH_HELPER]);
            command
        };
        // Never fall back to an interactive administrator prompt for Off.
        command.env("SUDO_ASKPASS", "/bin/false");
        self.execute(command, None).map_err(|e| format!("Could not turn SSH off without a password: {e} Run Connections > Prepare this host once to install the limited shutdown permission, then retry."))
    }
    #[cfg(unix)]
    fn request_host_password(&self, purpose: &str) -> Result<String, String> {
        let (response, receiver) = mpsc::channel();
        self.events
            .send(Event::HostPassword {
                purpose: purpose.into(),
                response,
            })
            .map_err(|_| "Setup window closed.".to_owned())?;
        self.ctx.request_repaint();
        wait_for_host_password(receiver, self.cancelled)
    }
    #[cfg(unix)]
    fn setup_host(&mut self, script: &str, purpose: &str, mode: OutputMode) -> Result<(), String> {
        if unsafe { libc::geteuid() } == 0 {
            let mut command = Command::new("sh");
            command.args(["-c", script]);
            return self.execute_output(command, None, mode).map(|_| ());
        }
        if available("sudo") {
            let mut check = Command::new("sudo");
            check.args(["-n", "-v"]);
            let cached = self
                .execute_output(check, None, OutputMode::Capture)
                .is_ok();
            if self.cancelled.load(Ordering::Relaxed) {
                return Err("Stopped.".into());
            }
            let password = if cached {
                None
            } else {
                Some(self.request_host_password(purpose)?)
            };
            let mut command = Command::new("sudo");
            if password.is_some() {
                command.args(["-S", "-p", ""]);
            } else {
                command.arg("-n");
            }
            command.args(["--", "sh", "-c", script]);
            return self
                .execute_output(command, password.as_deref(), mode)
                .map(|_| ())
                .map_err(|e| format!("Host setup ({purpose}): {e}"));
        }
        if available("pkexec") {
            let mut command = Command::new("pkexec");
            command.args(["--disable-internal-agent", "sh", "-c", script]);
            return self.execute_output(command, None, mode)
                .map(|_| ()).map_err(|e| format!("Host setup ({purpose}): {e} Install sudo to use the in-app password dialog, or start a desktop Polkit agent."));
        }
        Err("Host setup requires sudo or pkexec. Install sudo, then retry host setup.".into())
    }
    #[cfg(windows)]
    fn disable_host(&mut self) -> Result<(), String> {
        Err("Incoming SSH setup is supported on Linux only.".into())
    }
    #[cfg(windows)]
    fn setup_host(
        &mut self,
        _script: &str,
        purpose: &str,
        _mode: OutputMode,
    ) -> Result<(), String> {
        Err(format!(
            "For {purpose}, install and sign in to the Windows Tailscale app, then retry. Incoming SSH setup is Linux-only."
        ))
    }
    fn setup_tailscale(&mut self) -> Result<(), String> {
        let script = include_str!("tailscale-setup.sh");
        let local_running = if available("tailscale") {
            let mut status = Command::new("tailscale");
            status.args(["status", "--json"]);
            self.execute_output(status, None, OutputMode::Capture)
                .is_ok_and(|out| tailscale_running(&out))
        } else {
            false
        };
        if local_running {
            let _ = self.events.send(Event::Output(
                b"This host is already connected to Tailscale.\n".to_vec(),
            ));
        } else {
            let _ = self.events.send(Event::Output(
                b"Preparing Tailscale on this host...\n".to_vec(),
            ));
            self.setup_host(script, "Tailscale", OutputMode::Tailscale("This host"))?;
        }
        let server_running = self
            .execute_output(
                ssh(&self.task.server, "tailscale status --json"),
                None,
                OutputMode::Capture,
            )
            .is_ok_and(|out| tailscale_running(&out));
        if !server_running {
            let _ = self.events.send(Event::Output(
                b"Preparing Tailscale on the server with its saved administrator credentials...\n"
                    .to_vec(),
            ));
            self.execute_output(
                ssh(&self.task.server, &privileged_remote(script)),
                Some(self.task.credentials.sudo_password()),
                OutputMode::Tailscale("Server"),
            )
            .map_err(|e| format!("Server Tailscale setup: {e}"))?;
        } else {
            let _ = self.events.send(Event::Output(
                b"The server is already connected to Tailscale.\n".to_vec(),
            ));
        }
        let status = self.execute_output(
            ssh(&self.task.server, "tailscale status --json"),
            None,
            OutputMode::Capture,
        )?;
        let host = tailscale_address(&status)?;
        let port_text = self.execute_output(
            ssh(&self.task.server, "printf '%s' \"${SSH_CONNECTION##* }\""),
            None,
            OutputMode::Capture,
        )?;
        let port = port_text
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|p| *p != 0)
            .ok_or("Cannot determine the server's SSH listening port.")?;
        let mut server = self.task.server.clone();
        server.host = host.clone();
        server.port = port;
        let _ = self.events.send(Event::Output(
            b"Verifying SSH over Tailscale using the saved server identity...\n".to_vec(),
        ));
        self.execute_output(ssh(&server, "true"), None, OutputMode::Capture)
            .map_err(|_| "Tailscale is installed, but SSH over it could not be verified. Check that both devices are in the same Tailscale network and its access rules allow SSH. Your existing addresses were kept; retry setup after correcting access.".to_owned())?;
        let _ = self.events.send(Event::TailscaleReady {
            server: server.id,
            host,
            port,
        });
        self.ctx.request_repaint();
        Ok(())
    }
}

#[derive(Default)]
pub struct Output {
    pub text: String,
    escape: u8,
}
impl Output {
    pub fn append(&mut self, bytes: &[u8]) {
        for c in String::from_utf8_lossy(bytes).chars() {
            match self.escape {
                1 => {
                    self.escape = match c {
                        '[' => 2,
                        ']' => 3,
                        _ => 0,
                    }
                }
                2 => {
                    if ('@'..='~').contains(&c) {
                        self.escape = 0;
                    }
                }
                3 => {
                    if c == '\x07' {
                        self.escape = 0;
                    } else if c == '\x1b' {
                        self.escape = 4;
                    }
                }
                4 => self.escape = if c == '\\' { 0 } else { 3 },
                _ => match c {
                    '\x1b' => self.escape = 1,
                    '\x08' => {
                        self.text.pop();
                    }
                    '\n' | '\t' => self.text.push(c),
                    c if !c.is_control() => self.text.push(c),
                    _ => {}
                },
            }
        }
        if self.text.len() > 200_000 {
            let mut cut = self.text.len() - 150_000;
            while !self.text.is_char_boundary(cut) {
                cut += 1;
            }
            self.text.drain(..cut);
        }
    }
}

fn local_host_setup_script() -> String {
    #[cfg(unix)]
    {
        host_setup_script(unsafe { libc::getuid() })
    }
    #[cfg(windows)]
    {
        String::new()
    }
}
fn stop_child(child: &mut std::process::Child, force: bool) {
    #[cfg(unix)]
    unsafe {
        libc::kill(
            -(child.id() as i32),
            if force { libc::SIGKILL } else { libc::SIGTERM },
        );
    }
    #[cfg(windows)]
    {
        let _ = force;
        let _ = Command::new("taskkill.exe")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .status();
        let _ = child.kill();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn tailscale_matching_uses_peer_addresses_and_unique_names_not_lan_guesses() {
        let status = r#"{"BackendState":"Running","Peer":{"a":{"HostName":"workstation","DNSName":"workstation.example.ts.net.","TailscaleIPs":["fd7a:115c:a1e0::1","100.100.1.2"]}}}"#;
        for configured in [
            "workstation",
            "WORKSTATION.EXAMPLE.TS.NET.",
            "100.100.1.2",
            "[fd7a:115c:a1e0::1]",
        ] {
            assert_eq!(
                peer_address(status, configured).as_deref(),
                Some("100.100.1.2")
            );
        }
        assert!(peer_address(status, "192.168.1.2").is_none());
        assert!(peer_address(&status.replace("Running", "Stopped"), "workstation").is_none());
        let duplicate = r#"{"BackendState":"Running","Peer":{"a":{"HostName":"same","TailscaleIPs":["100.100.1.2"]},"b":{"HostName":"same","TailscaleIPs":["100.100.1.3"]}}}"#;
        assert!(peer_address(duplicate, "same").is_none());
        assert!(!tailscale_membership(status, "100.100.9.9"));
    }

    #[test]
    fn passwordless_stop_policy_is_limited_to_one_helper_without_arguments() {
        let policy = stop_ssh_policy(1000);
        assert!(policy.contains(
            "#1000 ALL=(root) NOPASSWD: NOSETENV: /usr/local/libexec/codesync-stop-ssh \"\""
        ));
        assert!(!policy.contains("ALL=(ALL)"));
        let scratch = super::super::sync::Scratch::new().unwrap();
        let path = scratch.0.join("sudoers");
        std::fs::write(&path, policy).unwrap();
        if available("visudo") {
            assert!(
                Command::new("visudo")
                    .arg("-cf")
                    .arg(path)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        for script in [stop_ssh_helper(), host_setup_script(1000)] {
            assert!(
                Command::new("sh")
                    .args(["-n", "-c", &script])
                    .status()
                    .unwrap()
                    .success()
            );
        }
        assert!(
            Command::new("sh")
                .args(["-c", &stop_ssh_helper(), "helper", "unexpected"])
                .status()
                .unwrap()
                .code()
                == Some(2)
        );
    }

    #[test]
    fn sync_group_end_to_end_with_local_ssh_transport() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = super::super::sync::Scratch::new().unwrap();
        if std::env::var_os("CODESYNC_TEST_TRANSPORT").is_none() {
            let wrapper = scratch.0.join("ssh");
            std::fs::write(&wrapper, "#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do if [ \"$1\" = sync-test-host ]; then shift; exec sh -c \"$*\"; fi; shift; done\nexit 1\n").unwrap();
            std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
            let mut paths = vec![scratch.0.clone()];
            paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "jobs::tests::sync_group_end_to_end_with_local_ssh_transport",
                    "--nocapture",
                ])
                .env("CODESYNC_TEST_TRANSPORT", "1")
                .env("PATH", std::env::join_paths(paths).unwrap())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let local = scratch.0.join("local");
        std::fs::create_dir(&local).unwrap();
        std::fs::write(local.join("conflict.txt"), "local version").unwrap();
        std::fs::write(local.join("local-only"), "local").unwrap();
        let mut tasks = Vec::new();
        for index in 0..2 {
            let remote = scratch.0.join(format!("remote{index}"));
            std::fs::create_dir(&remote).unwrap();
            std::fs::write(
                remote.join("conflict.txt"),
                format!("remote version {index}"),
            )
            .unwrap();
            std::fs::write(remote.join(format!("remote-only-{index}")), "remote").unwrap();
            let mut task = task();
            task.local = local.clone();
            task.remote = remote.to_str().unwrap().into();
            task.server.host = "sync-test-host".into();
            task.server.user.clear();
            task.server.name = format!("Server{index}");
            task.action = Action::Sync;
            tasks.push(task);
        }
        let mut bridge = Bridge::new().unwrap();
        let cancelled = AtomicBool::new(false);
        let (events, _receiver) = mpsc::channel();
        let ctx = eframe::egui::Context::default();
        sync_groups(tasks.clone(), &mut bridge, &cancelled, &events, &ctx).unwrap();
        let expected = super::super::sync::scan(&local, &cancelled).unwrap().files;
        assert_eq!(expected.len(), 6);
        assert!(!local.join("conflict.txt").exists());
        for task in &tasks {
            assert_eq!(
                super::super::sync::scan(std::path::Path::new(&task.remote), &cancelled)
                    .unwrap()
                    .files,
                expected
            );
        }
        sync_groups(tasks, &mut bridge, &cancelled, &events, &ctx).unwrap();
        assert_eq!(
            super::super::sync::scan(&local, &cancelled).unwrap().files,
            expected
        );
        assert!(
            Command::new("sh")
                .args(["-n", "-c", include_str!("host-setup.sh")])
                .status()
                .unwrap()
                .success()
        );
    }

    #[test]
    fn remote_conflict_rename_checks_hash_and_preserves_collisions() {
        let scratch = super::super::sync::Scratch::new().unwrap();
        let from = scratch.0.join("file with ' quote.txt");
        std::fs::write(&from, "original").unwrap();
        let rename = super::super::sync::Rename {
            participant: 1,
            from: "file with ' quote.txt".into(),
            to: "Server_file.txt".into(),
            hash: super::super::sync::hash(&from).unwrap(),
        };
        let script = remote_rename_script(scratch.0.to_str().unwrap(), &rename).unwrap();
        let execute = || {
            Command::new("sh")
                .args(["-c", &script])
                .output()
                .unwrap()
                .status
                .success()
        };
        std::fs::write(&from, "edited").unwrap();
        assert!(!execute());
        assert!(from.exists());
        std::fs::write(&from, "original").unwrap();
        std::fs::write(scratch.0.join(&rename.to), "keep").unwrap();
        assert!(!execute());
        assert_eq!(
            std::fs::read_to_string(scratch.0.join(&rename.to)).unwrap(),
            "keep"
        );
        std::fs::remove_file(scratch.0.join(&rename.to)).unwrap();
        assert!(execute());
        assert!(!from.exists());
        assert_eq!(
            std::fs::read_to_string(scratch.0.join(&rename.to)).unwrap(),
            "original"
        );
    }

    #[test]
    fn host_authorization_handles_submit_cancel_and_stop() {
        let cancelled = AtomicBool::new(false);
        let (tx, rx) = mpsc::channel();
        tx.send(Some("test-password".into())).unwrap();
        assert_eq!(
            wait_for_host_password(rx, &cancelled).unwrap(),
            "test-password"
        );
        let (tx, rx) = mpsc::channel();
        tx.send(None).unwrap();
        assert!(
            wait_for_host_password(rx, &cancelled)
                .unwrap_err()
                .contains("cancelled")
        );
        let (_tx, rx) = mpsc::channel();
        cancelled.store(true, Ordering::Relaxed);
        assert!(
            wait_for_host_password(rx, &cancelled)
                .unwrap_err()
                .contains("Stopped")
        );
        assert!(tailscale_running(r#"{"BackendState":"Running"}"#));
        assert!(!tailscale_running(r#"{"BackendState":"NeedsLogin"}"#));
        assert!(
            failure_reason("ssh: connect to host private.example port 22: Connection refused")
                .contains("port refused")
        );
        assert!(!failure_reason("private.example: Permission denied").contains("private.example"));
    }

    #[test]
    fn all_endpoints_are_bound_to_the_same_server_identity() {
        let mut task = task();
        let local = ssh_args(&task.server);
        task.server.host = "public.example.com".into();
        task.server.port = 2200;
        let remote = ssh_args(&task.server);
        for args in [&local, &remote] {
            assert!(args.contains(&"StrictHostKeyChecking=yes".into()));
            assert!(args.contains(&"HostKeyAlias=codesync-server-1".into()));
            assert!(args.contains(&"UserKnownHostsFile=~/.ssh/codesync_known_hosts".into()));
            assert!(args.contains(&"ControlPath=~/.ssh/codesync-server-1-%C".into()));
        }
        task.server.id = 2;
        assert!(ssh_args(&task.server).contains(&"HostKeyAlias=codesync-server-2".into()));
    }
    #[test]
    fn tailscale_login_and_status_are_handled_without_showing_private_details() {
        assert_eq!(
            login_url("Visit https://login.tailscale.com/a/test123\n").as_deref(),
            Some("https://login.tailscale.com/a/test123")
        );
        assert!(login_url("https://login.tailscale.com.evil.example/a/test").is_none());
        let (sender, receiver) = mpsc::channel();
        emit_tailscale_line(
            b"https://login.tailscale.com/a/test123\n",
            "Server",
            &sender,
        );
        assert!(matches!(receiver.recv().unwrap(), Event::Login { .. }));
        if let Event::Output(bytes) = receiver.recv().unwrap() {
            assert!(!String::from_utf8_lossy(&bytes).contains("test123"));
        } else {
            panic!("expected generic activity message");
        }
        assert_eq!(tailscale_address(r#"{"BackendState":"Running","Self":{"TailscaleIPs":["fd7a:115c:a1e0::1","100.100.1.2"]}}"#).unwrap(), "100.100.1.2");
        assert!(
            tailscale_address(
                r#"{"BackendState":"NeedsLogin","Self":{"TailscaleIPs":["100.100.1.2"]}}"#
            )
            .is_err()
        );
        assert!(
            Command::new("sh")
                .args(["-n", "-c", include_str!("tailscale-setup.sh")])
                .status()
                .unwrap()
                .success()
        );
    }
    fn task() -> Task {
        Task {
            server: Server {
                id: 1,
                name: "Test".into(),
                host: "host".into(),
                user: "user".into(),
                port: 2222,
                network: Default::default(),
            },
            credentials: Credentials::default(),
            local: std::env::temp_dir(),
            remote: "/home/user/class notes".into(),
            action: Action::Test,
        }
    }
    #[test]
    fn sync_routes_each_mapping_without_touching_folder_config() {
        let mut task = task();
        let cmd = sync_command(&task, false, true);
        let args: Vec<_> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            &args[args.len() - 2..],
            ["./", "user@host:/home/user/class notes/"]
        );
        assert!(args.contains(&"--dry-run".to_owned()));
        assert!(args.iter().any(|a| a.contains("2222")));
        task.server.host = "other".into();
        task.remote = "/backup".into();
        let cmd = sync_command(&task, true, false);
        let args: Vec<_> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(&args[args.len() - 2..], ["user@other:/backup/", "./"]);
    }
    #[test]
    fn runner_reports_failure_and_cancels_process_group() {
        let task = task();
        let mut bridge = Bridge::new().unwrap();
        let (events, _rx) = mpsc::channel();
        let ctx = eframe::egui::Context::default();
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut runner = Runner {
            task: &task,
            bridge: &mut bridge,
            cancelled: &cancelled,
            events: &events,
            ctx: &ctx,
        };
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "exit 7"]);
        assert!(runner.execute(cmd, None).unwrap_err().contains("7"));
        let signal = cancelled.clone();
        let timer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            signal.store(true, Ordering::Relaxed);
        });
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 30"]);
        assert!(runner.execute(cmd, None).unwrap_err().contains("Stopped"));
        timer.join().unwrap();
    }
    #[test]
    fn setup_password_is_sent_only_through_stdin() {
        let task = task();
        let mut bridge = Bridge::new().unwrap();
        let (events, output) = mpsc::channel();
        let ctx = eframe::egui::Context::default();
        let cancelled = AtomicBool::new(false);
        let mut runner = Runner {
            task: &task,
            bridge: &mut bridge,
            cancelled: &cancelled,
            events: &events,
            ctx: &ctx,
        };
        let mut cmd = Command::new("sh");
        cmd.args([
            "-c",
            "IFS= read -r password; test -n \"$password\" && printf ready",
        ]);
        let secret = "dummy-sudo-password";
        assert!(!format!("{cmd:?}").contains(secret));
        runner.execute(cmd, Some(secret)).unwrap();
        let mut text = String::new();
        for event in output.try_iter() {
            if let Event::Output(bytes) = event {
                text.push_str(&String::from_utf8_lossy(&bytes));
            }
        }
        assert_eq!(text, "ready");
        let mut syntax = Command::new("sh");
        assert!(
            syntax
                .args(["-n", "-c", &setup_script()])
                .status()
                .unwrap()
                .success()
        );
    }
    #[test]
    fn strips_terminal_sequences_across_reads() {
        let mut output = Output::default();
        output.append(b"\x1b[3");
        output.append(b"1mHello\x1b[0m\r\n");
        assert_eq!(output.text, "Hello\n");
    }
}
