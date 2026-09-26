//! Host-side paths and tools. Remote paths always use Linux syntax.
use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// Cygwin supplies a matched SSH/rsync/coreutils toolchain on Windows.
pub fn command(program: &str) -> Command {
    #[cfg(windows)]
    {
        let bin = std::env::var_os("CODESYNC_CYGWIN_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\cygwin64\bin"));
        let mut command = Command::new(bin.join(format!("{program}.exe")));
        // Do not let emulated Cygwin links masquerade as regular Windows files.
        command.env(
            "CYGWIN",
            format!(
                "{} winsymlinks:nativestrict",
                std::env::var("CYGWIN").unwrap_or_default()
            ),
        );
        if let Some(home) = home() {
            command.env("HOME", local_path(&home));
        }
        let mut paths = vec![bin];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        if let Ok(path) = std::env::join_paths(paths) {
            command.env("PATH", path);
        }
        command
    }
    #[cfg(not(windows))]
    Command::new(program)
}

pub fn local_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    if cfg!(windows) {
        windows_path(&path)
    } else {
        path.into_owned()
    }
}

fn windows_path(path: &str) -> String {
    let path = path
        .strip_prefix(r"\\?\UNC\")
        .map(|p| format!(r"\\{p}"))
        .unwrap_or_else(|| path.strip_prefix(r"\\?\").unwrap_or(path).to_owned());
    let path = path.replace('\\', "/");
    let bytes = path.as_bytes();
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && &bytes[1..3] == b":/" {
        format!(
            "/cygdrive/{}/{}",
            (bytes[0] as char).to_ascii_lowercase(),
            &path[3..]
        )
    } else {
        path
    }
}

pub fn private_directory(path: &Path, recursive: bool) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(recursive);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    // Windows directories inherit the user's profile/temp directory ACL.
    builder.create(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn converts_windows_paths_without_touching_remote_syntax() {
        for (input, expected) in [
            (
                r"C:\Users\Some One\notes",
                "/cygdrive/c/Users/Some One/notes",
            ),
            (r"\\?\D:\notes\世界", "/cygdrive/d/notes/世界"),
            (r"\\server\share\notes", "//server/share/notes"),
            (r"\\?\UNC\server\share", "//server/share"),
            ("./", "./"),
        ] {
            assert_eq!(windows_path(input), expected);
        }
    }
}

/// Reject names that Win32 aliases or cannot represent before fetching a Linux tree.
pub fn validate_windows_paths<'a>(paths: impl IntoIterator<Item = &'a str>) -> Result<(), String> {
    let mut seen = std::collections::BTreeMap::new();
    for path in paths {
        let path = path.trim_start_matches("./");
        if path.is_empty() || path == "." {
            continue;
        }
        let mut prefix = String::new();
        for component in path.split('/') {
            let stem = component.split('.').next().unwrap_or("").to_uppercase();
            let device = matches!(
                stem.as_str(),
                "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
            ) || ["COM", "LPT"].iter().any(|base| {
                stem.strip_prefix(base).is_some_and(|n| {
                    matches!(
                        n,
                        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                    )
                })
            });
            if component.is_empty()
                || component == "."
                || component == ".."
                || component.ends_with(['.', ' '])
                || device
                || component
                    .chars()
                    .any(|c| c.is_control() || "<>:\"\\|?*\u{fffd}".contains(c))
            {
                return Err(format!(
                    "Linux path {path:?} cannot be safely represented on Windows. Rename it on the server first."
                ));
            }
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(component);
            if let Some(previous) = seen.insert(prefix.to_uppercase(), prefix.clone())
                && previous != prefix
            {
                return Err(format!(
                    "Paths {previous:?} and {prefix:?} differ only by case. Rename one before syncing to Windows."
                ));
            }
        }
    }
    Ok(())
}

pub fn windows_manifest_script(root: &str) -> String {
    let excludes = crate::EXCLUDES
        .iter()
        .map(|p| format!("-name {}", crate::quote(p)))
        .collect::<Vec<_>>()
        .join(" -o ");
    format!(
        "set -eu; cd -- {}; special=$(find . \\( {excludes} \\) -prune -o ! -type f ! -type d -print); test -z \"$special\" || {{ echo 'Windows sync requires regular files and directories.' >&2; exit 1; }}; find . \\( {excludes} \\) -prune -o -print0; printf 'CODESYNC-END\\0'",
        crate::quote(root)
    )
}

pub fn validate_windows_manifest(manifest: &str) -> Result<(), String> {
    let paths = manifest
        .strip_suffix("CODESYNC-END\0")
        .ok_or("Cannot validate the complete Linux file list; no files transferred.")?;
    validate_windows_paths(paths.split('\0').filter(|s| !s.is_empty()))
}

#[cfg(test)]
mod windows_name_tests {
    use super::*;
    #[test]
    fn rejects_aliases_reserved_names_and_incomplete_manifests() {
        for paths in [
            vec!["foo", "FOO"],
            vec!["Dir/a", "dir/b"],
            vec!["aux.txt"],
            vec!["a:b"],
            vec!["a\\b"],
            vec!["a."],
            vec!["a "],
            vec!["COM1.txt"],
            vec!["../escape"],
        ] {
            assert!(validate_windows_paths(paths).is_err());
        }
        assert!(validate_windows_paths(["notes/世界.txt", "notes/other.txt"]).is_ok());
        assert!(validate_windows_manifest("./\0./notes\0CODESYNC-END\0").is_ok());
        assert!(validate_windows_manifest("./notes\0").is_err());
    }
}

pub fn rsync_options(command: &mut Command) {
    if cfg!(windows) {
        command.args(["--no-owner", "--no-group", "--no-perms", "--omit-dir-times"]);
    }
}

#[cfg(all(test, unix))]
mod manifest_tests {
    use super::*;
    #[test]
    fn linux_manifest_checks_names_and_types_and_prunes_exclusions() {
        let root =
            std::env::temp_dir().join(format!("codesync-manifest-test-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let run = || {
            Command::new("sh")
                .args(["-c", &windows_manifest_script(root.to_str().unwrap())])
                .output()
                .unwrap()
        };
        std::fs::create_dir(root.join(".git")).unwrap();
        std::os::unix::fs::symlink("missing", root.join(".git/link")).unwrap();
        std::fs::write(root.join("normal ' file"), "content").unwrap();
        let output = run();
        assert!(output.status.success());
        assert!(validate_windows_manifest(std::str::from_utf8(&output.stdout).unwrap()).is_ok());
        std::fs::write(root.join("NORMAL ' FILE"), "different").unwrap();
        assert!(validate_windows_manifest(std::str::from_utf8(&run().stdout).unwrap()).is_err());
        std::os::unix::fs::symlink("missing", root.join("link")).unwrap();
        assert!(!run().status.success());
        std::fs::remove_dir_all(root).unwrap();
    }
}
