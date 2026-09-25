use std::{
    env, fs,
    process::{Command, ExitCode},
};

const HELP: &str = "codesync — sync code and notes over SSH
  codesync init USER@HOST /absolute/remote/folder  Save settings and set up server
  codesync setup                                Retry server setup
  codesync push [--dry-run]
  codesync pull [--dry-run]
  codesync run COMMAND [ARGS...]
  codesync shell
Run from your project folder. Sync may overwrite files; preview with --dry-run.";

const SETUP: &str = include_str!("setup.sh");

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
struct Config {
    host: String,
    dir: String,
}
impl Config {
    fn validate(&self) -> Result<(), String> {
        if self.host.is_empty()
            || self.host.starts_with('-')
            || !self
                .host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "@._-".contains(c))
        {
            return Err(
                "Use user@hostname or an SSH config alias, without options or ports.".into(),
            );
        }
        if !self.dir.starts_with('/')
            || self.dir.trim_end_matches('/').is_empty()
            || self.dir.chars().any(char::is_control)
        {
            return Err("Remote directory must be an absolute path other than /.".into());
        }
        Ok(())
    }
    fn read() -> Result<Self, String> {
        let content = fs::read_to_string(".codesync")
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
        if interactive {
            cmd.arg("-t");
        }
        execute(cmd.arg(&self.host).arg(script))
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
        cmd.args(["-az", "--protect-args", "--itemize-changes", "-e", "ssh"]);
        for pattern in [
            ".git",
            "target",
            "node_modules",
            ".codesync",
            ".env",
            ".env.*",
            "*.pem",
            "*.key",
        ] {
            cmd.arg(format!("--exclude={pattern}"));
        }
        if dry {
            cmd.arg("--dry-run");
        }
        let remote = format!("{}:{}/", self.host, self.dir.trim_end_matches('/'));
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
fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(action) = args.first().map(String::as_str) else {
        println!("{HELP}");
        return Ok(());
    };
    match action {
        "help" | "--help" | "-h" => println!("{HELP}"),
        "init" => {
            if args.len() != 3 {
                return Err("Usage: codesync init USER@HOST /absolute/remote/folder".into());
            }
            let config = Config {
                host: args[1].clone(),
                dir: args[2].clone(),
            };
            config.validate()?;
            use std::io::Write;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(".codesync")
                .map_err(|e| format!("Cannot create .codesync: {e}"))?;
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
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("codesync: {error}");
            ExitCode::FAILURE
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
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
