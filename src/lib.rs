use std::{env, fs, process::Command};
mod update;

const HELP: &str = "codesync — sync code and notes over SSH
  codesync init [--force] USER@HOST /absolute/remote/folder
  codesync gui                                  Open the desktop app
  codesync update [--repo /path/to/codesync]      Pull and reinstall from Git
  codesync setup                                Retry server setup
  codesync push [--dry-run]
  codesync pull [--dry-run]
  codesync run COMMAND [ARGS...]
  codesync shell
Run sync commands from your project folder. Sync may overwrite files; preview with --dry-run.";

pub const EXCLUDES: [&str; 8] = [
    ".git",
    "target",
    "node_modules",
    ".codesync",
    ".env",
    ".env.*",
    "*.pem",
    "*.key",
];

const SETUP: &str = include_str!("setup.sh");
// Shared by direct SSH commands and rsync's SSH transport.
const SSH_OPTIONS: [&str; 6] = [
    "-o",
    "ControlMaster=auto",
    "-o",
    "ControlPersist=10m",
    "-o",
    "ControlPath=~/.ssh/codesync-%C",
];

fn ssh_transport() -> String {
    format!("ssh {}", SSH_OPTIONS.join(" "))
}

pub fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn host_parts(destination: &str) -> (&str, &str) {
    destination.rsplit_once('@').unwrap_or(("", destination))
}
fn bare_host(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host)
}
pub fn ssh_destination(destination: &str) -> String {
    let (user, host) = host_parts(destination);
    let host = bare_host(host);
    if user.is_empty() {
        host.into()
    } else {
        format!("{user}@{host}")
    }
}
pub fn rsync_destination(destination: &str, directory: &str) -> String {
    let (user, host) = host_parts(destination);
    let host = bare_host(host);
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.into()
    };
    let prefix = if user.is_empty() {
        host
    } else {
        format!("{user}@{host}")
    };
    format!("{prefix}:{}/", directory.trim_end_matches('/'))
}
pub struct Config {
    pub host: String,
    pub dir: String,
}
impl Config {
    pub fn validate(&self) -> Result<(), String> {
        let (user, host) = host_parts(&self.host);
        let simple = |s: &str| {
            !s.is_empty()
                && !s.starts_with('-')
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        };
        let valid_user = !self.host.contains('@') || simple(user);
        let valid_host = simple(host) || bare_host(host).parse::<std::net::Ipv6Addr>().is_ok();
        if !valid_user || !valid_host {
            return Err(
                "Use an IPv4/IPv6 address, hostname, or SSH alias, optionally prefixed with user@. Configure the port separately.".into(),
            );
        }
        let home_codesync = self.dir == "codesync"
            || self.dir.strip_prefix("codesync/").is_some_and(|rest| {
                !rest.is_empty()
                    && rest
                        .split('/')
                        .all(|part| !part.is_empty() && part != "." && part != "..")
            });
        if (!self.dir.starts_with('/') && !home_codesync)
            || self.dir.trim_end_matches('/').is_empty()
            || self.dir.chars().any(char::is_control)
        {
            return Err("Remote directory must be an absolute path other than /, or codesync (optionally with subfolders) in the server user's home directory.".into());
        }
        Ok(())
    }
    fn read() -> Result<Self, String> {
        Self::read_at(std::path::Path::new("."))
    }
    pub fn read_at(folder: &std::path::Path) -> Result<Self, String> {
        let content = fs::read_to_string(folder.join(".codesync"))
            .map_err(|e| format!("Cannot read .codesync: {e}. Run codesync init first."))?;
        let lines: Vec<_> = content.lines().collect();
        if lines.len() != 2 {
            return Err("Invalid .codesync: expected host and directory on separate lines.".into());
        }
        let config = Self {
            host: lines[0].into(),
            dir: lines[1].into(),
        };
        config.validate()?;
        Ok(config)
    }
    fn ssh(&self, script: &str, interactive: bool) -> Result<(), String> {
        let mut cmd = Command::new("ssh");
        cmd.args(SSH_OPTIONS);
        if interactive {
            cmd.arg("-t");
        }
        execute(cmd.arg(ssh_destination(&self.host)).arg(script))
    }
    fn setup(&self) -> Result<(), String> {
        println!("Checking server dependencies on {}...", self.host);
        self.ssh(&format!("sh -c {}", quote(SETUP)), true)
            .map_err(|e| {
                format!("Server setup failed: {e}. Settings are saved; retry with codesync setup.")
            })
    }
    fn sync(&self, pull: bool, dry: bool) -> Result<(), String> {
        if !pull && !dry {
            self.ssh(&format!("mkdir -p -- {}", quote(&self.dir)), false)?;
        }
        let mut cmd = Command::new("rsync");
        cmd.args(["-az", "--protect-args", "--itemize-changes", "-e"])
            .arg(ssh_transport());
        for pattern in EXCLUDES {
            cmd.arg(format!("--exclude={pattern}"));
        }
        if dry {
            cmd.arg("--dry-run");
        }
        let remote = rsync_destination(&self.host, &self.dir);
        cmd.arg("--");
        if pull {
            cmd.arg(remote).arg("./");
        } else {
            cmd.arg("./").arg(remote);
        }
        execute(&mut cmd)
    }
}
fn execute(cmd: &mut Command) -> Result<(), String> {
    let name = cmd.get_program().to_string_lossy().into_owned();
    let status = cmd
        .status()
        .map_err(|e| format!("Cannot start {name}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{name} failed ({status})"))
    }
}
pub fn run_cli(args: Vec<String>) -> Result<(), String> {
    let Some(action) = args.first().map(String::as_str) else {
        println!("{HELP}");
        return Ok(());
    };
    match action {
        "help" | "--help" | "-h" => println!("{HELP}"),
        "update" => update::run(&args[1..])?,
        "gui" => {
            if args.len() != 1 {
                return Err("Usage: codesync gui".into());
            }
            let executable = env::current_exe()
                .map_err(|e| e.to_string())?
                .with_file_name("codesync-gui");
            execute(&mut Command::new(executable))?;
        }
        "init" => {
            let (force, host, dir) = match &args[1..] {
                [host, dir] => (false, host, dir),
                [flag, host, dir] if flag == "--force" => (true, host, dir),
                _ => {
                    return Err(
                        "Usage: codesync init [--force] USER@HOST /absolute/remote/folder".into(),
                    );
                }
            };
            let config = Config {
                host: host.clone(),
                dir: dir.clone(),
            };
            config.validate()?;
            use std::io::Write;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(!force)
                .create(force)
                .truncate(force)
                .open(".codesync")
                .map_err(|e| {
                    if e.kind() == std::io::ErrorKind::AlreadyExists {
                        "This folder is already configured. Use codesync init --force USER@HOST /remote/folder to change its destination, or codesync setup to retry setup.".into()
                    } else {
                        format!("Cannot write .codesync: {e}")
                    }
                })?;
            writeln!(file, "{}\n{}", config.host, config.dir).map_err(|e| e.to_string())?;
            println!("Configured {}:{}", config.host, config.dir);
            config.setup()?;
        }
        "setup" => {
            if args.len() != 1 {
                return Err("Usage: codesync setup".into());
            }
            Config::read()?.setup()?;
        }
        "push" | "pull" => {
            if args.len() > 2 || (args.len() == 2 && args[1] != "--dry-run") {
                return Err(format!("Usage: codesync {action} [--dry-run]"));
            }
            Config::read()?.sync(action == "pull", args.len() == 2)?;
        }
        "run" => {
            if args.len() < 2 {
                return Err("Usage: codesync run COMMAND [ARGS...]".into());
            }
            let config = Config::read()?;
            config.sync(false, false)?;
            let command = args[1..]
                .iter()
                .map(|s| quote(s))
                .collect::<Vec<_>>()
                .join(" ");
            config.ssh(&format!("cd -- {} && {command}", quote(&config.dir)), false)?;
        }
        "shell" => {
            if args.len() != 1 {
                return Err("Usage: codesync shell".into());
            }
            let config = Config::read()?;
            config.ssh(
                &format!(
                    "mkdir -p -- {0} && cd -- {0} && exec \"${{SHELL:-/bin/sh}}\" -l",
                    quote(&config.dir)
                ),
                true,
            )?;
        }
        _ => return Err(format!("Unknown command: {action}. Use codesync --help.")),
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_addresses_use_correct_ssh_and_rsync_syntax() {
        for (input, ssh, rsync) in [
            (
                "user@203.0.113.8",
                "user@203.0.113.8",
                "user@203.0.113.8:/work/",
            ),
            (
                "user@server.example.com",
                "user@server.example.com",
                "user@server.example.com:/work/",
            ),
            (
                "user@2001:db8::8",
                "user@2001:db8::8",
                "user@[2001:db8::8]:/work/",
            ),
            (
                "user@[2001:db8::8]",
                "user@2001:db8::8",
                "user@[2001:db8::8]:/work/",
            ),
            ("2001:db8::8", "2001:db8::8", "[2001:db8::8]:/work/"),
        ] {
            assert!(
                Config {
                    host: input.into(),
                    dir: "/work".into()
                }
                .validate()
                .is_ok()
            );
            assert_eq!(ssh_destination(input), ssh);
            assert_eq!(rsync_destination(input, "/work/"), rsync);
        }
        for input in [
            "user@host:22",
            "user@203.0.113.8:22",
            "user@[broken]",
            "user@host@other",
            "@host",
            "user@",
            "user@::;id",
        ] {
            assert!(
                Config {
                    host: input.into(),
                    dir: "/work".into()
                }
                .validate()
                .is_err()
            );
        }
    }
    #[test]
    fn shell_arguments_roundtrip() {
        for input in ["", "hello world", "it's a note", "; $(whoami)\n*"] {
            let output = Command::new("sh")
                .arg("-c")
                .arg(format!("printf %s {}", quote(input)))
                .output()
                .unwrap();
            assert!(output.status.success());
            assert_eq!(String::from_utf8(output.stdout).unwrap(), input);
        }
    }
    #[test]
    fn reject_unsafe_destinations() {
        for (host, dir) in [
            ("-oProxyCommand=evil", "/work"),
            ("host;evil", "/work"),
            ("host", "/"),
            ("host", "relative"),
            ("host", "codesync/../other"),
            ("host", "codesync//other"),
            ("host", "/work\nother"),
        ] {
            assert!(
                Config {
                    host: host.into(),
                    dir: dir.into()
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            Config {
                host: "me@my-server".into(),
                dir: "/home/me/class notes".into()
            }
            .validate()
            .is_ok()
        );
    }
}
