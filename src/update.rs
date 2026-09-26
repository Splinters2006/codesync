use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

const INSTALLER: &str = if cfg!(windows) {
    "install.ps1"
} else {
    "install.sh"
};

const USAGE: &str = "Usage: codesync update [--repo /path/to/codesync]";

pub fn run(args: &[String]) -> Result<(), String> {
    let source = match args {
        [] => PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        [flag, path] if flag == "--repo" => PathBuf::from(path),
        _ => return Err(USAGE.into()),
    };
    let executable = env::current_exe().map_err(|e| e.to_string())?;
    let install_root = executable
        .parent()
        .filter(|parent| parent.file_name().is_some_and(|name| name == "bin"))
        .and_then(Path::parent);
    perform(&source, install_root, !cfg!(feature = "gui"))
}
fn git_output(repo: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| format!("Cannot start Git: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    String::from_utf8(output.stdout)
        .map(|s| s.trim().to_owned())
        .map_err(|_| "Git returned a non-UTF-8 repository path.".into())
}
fn validate_checkout(source: &Path) -> Result<PathBuf, String> {
    let repo = source.canonicalize().map_err(|_| "The original Codesync checkout is missing. Use codesync update --repo /path/to/codesync, or clone the repository again.".to_owned())?;
    let root = git_output(&repo, &["rev-parse", "--show-toplevel"]).map_err(|_| {
        "Updates need a Git checkout. Use codesync update --repo /path/to/codesync.".to_owned()
    })?;
    if Path::new(&root).canonicalize().map_err(|e| e.to_string())? != repo {
        return Err("Point --repo at the root of the Codesync repository.".into());
    }
    let manifest = fs::read_to_string(repo.join("Cargo.toml"))
        .map_err(|_| "The checkout has no Cargo.toml.".to_owned())?;
    if !manifest
        .lines()
        .any(|line| line.trim() == "name = \"codesync\"")
        || !repo.join("src/lib.rs").is_file()
        || !repo.join(INSTALLER).is_file()
    {
        return Err("This does not look like the Codesync source repository.".into());
    }
    Ok(repo)
}
fn perform(source: &Path, install_root: Option<&Path>, cli_only: bool) -> Result<(), String> {
    let repo = validate_checkout(source)?;
    if !git_output(
        &repo,
        &["status", "--porcelain", "--untracked-files=normal"],
    )?
    .is_empty()
    {
        return Err("The Codesync checkout has local changes or untracked files. Commit or stash them before updating; no files have been changed.".into());
    }
    git_output(&repo, &["symbolic-ref", "--quiet", "HEAD"]).map_err(|_| {
        "The checkout is on a detached commit. Switch to a branch before updating.".to_owned()
    })?;
    git_output(
        &repo,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    )
    .map_err(|_| {
        "The current branch has no upstream. Configure its Git tracking branch before updating."
            .to_owned()
    })?;
    println!("Updating Codesync from its Git checkout...");
    let status = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["pull", "--ff-only", "--no-rebase"])
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("Git pull failed. Resolve the reported network, authentication, or branch divergence issue and retry; installation was not started.".into());
    }
    validate_checkout(&repo)?;
    println!("Installing the updated Codesync...");
    #[cfg(not(windows))]
    let mut install = {
        let mut command = Command::new("sh");
        command.arg(repo.join(INSTALLER));
        command
    };
    #[cfg(windows)]
    let mut install = {
        let mut command = Command::new("powershell.exe");
        command
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(repo.join(INSTALLER))
            .arg("-WaitForProcess")
            .arg(std::process::id().to_string());
        command
    };
    install.current_dir(&repo);
    if cli_only {
        install.arg(if cfg!(windows) {
            "-CliOnly"
        } else {
            "--cli-only"
        });
    }
    if let Some(root) = install_root {
        install.env("CARGO_INSTALL_ROOT", root);
    }
    #[cfg(windows)]
    {
        install
            .spawn()
            .map_err(|e| format!("Cannot start installer: {e}"))?;
        println!(
            "The installer will continue when this command exits. Close the GUI before updating; watch the installer output for the result."
        );
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let status = install
            .status()
            .map_err(|e| format!("Cannot start installer: {e}"))?;
        if !status.success() {
            return Err("The repository was updated, but installation failed. Fix the installer error, then run codesync update again.".into());
        }
        println!("Codesync updated. Close and reopen the GUI to use the new version.");
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = env::temp_dir().join(format!(
                "codesync-update-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn git(path: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(path)
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    #[test]
    fn updates_from_any_directory_and_preserves_install_mode_and_root() {
        let temp = Temp::new();
        let origin = temp.0.join("origin");
        fs::create_dir(&origin).unwrap();
        git(&origin, &["init", "-b", "main"]);
        fs::create_dir(origin.join("src")).unwrap();
        fs::write(origin.join("src/lib.rs"), "").unwrap();
        fs::write(
            origin.join("Cargo.toml"),
            "[package]\nname = \"codesync\"\n",
        )
        .unwrap();
        fs::write(origin.join("install.sh"), "set -eu\nprintf '%s' \"${1-}\" > \"$CARGO_INSTALL_ROOT/mode\"\ncp payload \"$CARGO_INSTALL_ROOT/installed\"\n").unwrap();
        fs::write(origin.join("payload"), "v1").unwrap();
        git(&origin, &["add", "."]);
        git(&origin, &["commit", "-m", "Initial"]);
        let checkout = temp.0.join("checkout");
        git(
            &temp.0,
            &[
                "clone",
                origin.to_str().unwrap(),
                checkout.to_str().unwrap(),
            ],
        );
        let root = temp.0.join("custom install");
        fs::create_dir(&root).unwrap();
        fs::write(origin.join("payload"), "v2").unwrap();
        git(&origin, &["commit", "-am", "Update"]);
        perform(&checkout, Some(&root), true).unwrap();
        assert_eq!(fs::read_to_string(root.join("installed")).unwrap(), "v2");
        assert_eq!(fs::read_to_string(root.join("mode")).unwrap(), "--cli-only");
        fs::write(checkout.join("payload"), "local edits").unwrap();
        assert!(
            perform(&checkout, Some(&root), false)
                .unwrap_err()
                .contains("local changes")
        );
        assert_eq!(
            fs::read_to_string(checkout.join("payload")).unwrap(),
            "local edits"
        );
        git(&checkout, &["restore", "payload"]);
        fs::write(checkout.join("notes"), "untracked").unwrap();
        assert!(perform(&checkout, Some(&root), false).is_err());
        fs::remove_file(checkout.join("notes")).unwrap();
        git(&checkout, &["checkout", "--detach"]);
        assert!(
            perform(&checkout, Some(&root), false)
                .unwrap_err()
                .contains("detached")
        );
        git(&checkout, &["checkout", "main"]);
        fs::write(checkout.join("payload"), "local commit").unwrap();
        git(&checkout, &["commit", "-am", "Local"]);
        fs::write(origin.join("payload"), "remote commit").unwrap();
        git(&origin, &["commit", "-am", "Remote"]);
        assert!(
            perform(&checkout, Some(&root), false)
                .unwrap_err()
                .contains("Git pull failed")
        );
        assert_eq!(fs::read_to_string(root.join("installed")).unwrap(), "v2");
        assert_eq!(
            fs::read_to_string(checkout.join("payload")).unwrap(),
            "local commit"
        );
    }
}
