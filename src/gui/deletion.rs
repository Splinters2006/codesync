//! Explicit file deletion. Missing copies are successful no-ops so partial jobs can be retried.
use super::store::{self, Data};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Local {
        root: PathBuf,
        path: String,
    },
    Remote {
        server: u64,
        root: String,
        path: String,
    },
}

pub fn relative(path: &Path) -> Result<String, String> {
    let text = path.to_str().ok_or("Deletion requires a UTF-8 filename.")?;
    let text = if cfg!(windows) {
        text.replace('\\', "/")
    } else {
        text.into()
    };
    validate_relative(&text)?;
    Ok(text)
}
fn validate_relative(path: &str) -> Result<(), String> {
    if path.contains('\0')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err("Choose a file inside the shared folder; deleting a root or parent path is not allowed.".into());
    }
    Ok(())
}

/// Only infer copies from saved links. Never guess that two servers share a directory.
pub fn targets(data: &Data, source: Target) -> Result<Vec<Target>, String> {
    let mut result = vec![source.clone()];
    let mut folders = Vec::new();
    match &source {
        Target::Local { root, path } => {
            validate_relative(path)?;
            for folder in &data.folders {
                if folder.path == *root {
                    folders.push((folder.id, path.clone()));
                }
            }
        }
        Target::Remote { server, root, path } => {
            validate_relative(path)?;
            let full = format!("{}/{path}", root.trim_end_matches('/'));
            for link in data.links.iter().filter(|link| link.server == *server) {
                let folder = data
                    .folders
                    .iter()
                    .find(|folder| folder.id == link.folder)
                    .ok_or("Linked folder is missing.")?;
                let remote = store::remote_directory(&link.remote, &folder.path)?;
                if let Some(relative) =
                    full.strip_prefix(&format!("{}/", remote.trim_end_matches('/')))
                {
                    validate_relative(relative)?;
                    folders.push((folder.id, relative.to_owned()));
                }
            }
        }
    }
    for (id, path) in folders {
        let folder = data
            .folders
            .iter()
            .find(|folder| folder.id == id)
            .ok_or("Folder is missing.")?;
        push_unique(
            &mut result,
            Target::Local {
                root: folder.path.clone(),
                path: path.clone(),
            },
        );
        for link in data.links.iter().filter(|link| link.folder == id) {
            data.validate_link(link)?;
            let target = Target::Remote {
                server: link.server,
                root: store::remote_directory(&link.remote, &folder.path)?,
                path: path.clone(),
            };
            // The browser and a folder link may describe the same remote file using different roots.
            push_unique(&mut result, target);
        }
    }
    Ok(result)
}
fn push_unique(targets: &mut Vec<Target>, candidate: Target) {
    let same = |target: &Target| match (target, &candidate) {
        (
            Target::Remote {
                server: a,
                root: ar,
                path: ap,
            },
            Target::Remote {
                server: b,
                root: br,
                path: bp,
            },
        ) => {
            a == b
                && format!("{}/{ap}", ar.trim_end_matches('/'))
                    == format!("{}/{bp}", br.trim_end_matches('/'))
        }
        _ => target == &candidate,
    };
    if !targets.iter().any(same) {
        targets.push(candidate);
    }
}

/// Anchor each parent with cd, reject links, and unlink only a regular file in that directory.
/// Never use recursive deletion or interpolate an unquoted filename into shell syntax.
pub fn script(root: &str, path: &str, remove: bool) -> Result<String, String> {
    validate_relative(path)?;
    if root.is_empty() || root.contains('\0') {
        return Err("Missing deletion root.".into());
    }
    let mut script = format!(
        "set -eu\nfail() {{ printf '%s\\n' 'Refusing deletion: a path is a link, directory, or special file, or is outside the selected folder.' >&2; exit 1; }}\nmissing() {{ printf 'missing\\n'; exit 0; }}\nroot={}\ntest ! -L \"$root\" || fail\nif [ ! -e \"$root\" ]; then missing; fi\ntest -d \"$root\" || fail\ncd -P -- \"$root\"\nbase=$(pwd -P)\n",
        codesync::quote(root)
    );
    let parts: Vec<_> = path.split('/').collect();
    for parent in &parts[..parts.len() - 1] {
        let parent = codesync::quote(&format!("./{parent}"));
        script.push_str(&format!("test ! -L {parent} || fail\nif [ ! -e {parent} ]; then missing; fi\ntest -d {parent} || fail\ncd -P -- {parent}\ncase \"$(pwd -P)/\" in \"$base/\"*) ;; *) fail ;; esac\n"));
    }
    let file = codesync::quote(&format!("./{}", parts.last().unwrap()));
    script.push_str(&format!(
        "test ! -L {file} || fail\nif [ ! -e {file} ]; then missing; fi\ntest -f {file} || fail\ntest -w . && test -x . || fail\n"
    ));
    if remove {
        script.push_str(&format!("rm -- {file}\nprintf 'deleted\\n'\n"));
    } else {
        script.push_str("printf 'present\\n'\n");
    }
    Ok(script)
}

#[cfg(test)]
mod tests {
    use super::super::store::{Folder, Link, Server};
    use super::*;
    #[test]
    fn maps_saved_copies_and_respects_component_boundaries() {
        let data = Data {
            folders: vec![Folder {
                id: 1,
                name: "Notes".into(),
                path: "/work/notes".into(),
            }],
            servers: (2..=3)
                .map(|id| Server {
                    id,
                    name: format!("Server {id}"),
                    host: format!("server{id}"),
                    user: "user".into(),
                    port: 22,
                    network: Default::default(),
                })
                .collect(),
            links: vec![
                Link {
                    id: 4,
                    folder: 1,
                    server: 2,
                    remote: "codesync/notes".into(),
                },
                Link {
                    id: 5,
                    folder: 1,
                    server: 3,
                    remote: "/custom/notes".into(),
                },
            ],
        };
        let local = Target::Local {
            root: "/work/notes".into(),
            path: "sub/file.txt".into(),
        };
        let targets = super::targets(&data, local.clone()).unwrap();
        assert_eq!(targets.len(), 3);
        let remote = Target::Remote {
            server: 2,
            root: "codesync".into(),
            path: "notes/sub/file.txt".into(),
        };
        let copies = super::targets(&data, remote).unwrap();
        assert_eq!(copies.len(), 3);
        assert!(copies.contains(&local));
        assert!(copies.contains(&targets[2]));
        let unrelated = Target::Remote {
            server: 2,
            root: "codesync".into(),
            path: "notes-other/file.txt".into(),
        };
        assert_eq!(super::targets(&data, unrelated).unwrap().len(), 1);
    }
    #[test]
    fn deletion_is_explicit_bounded_quoted_and_retryable() {
        let scratch = super::super::sync::Scratch::new().unwrap();
        std::fs::create_dir(scratch.0.join("sub")).unwrap();
        let name = "sub/-it's a file; $(echo bad).txt";
        std::fs::write(scratch.0.join(name), "delete me").unwrap();
        std::fs::write(scratch.0.join("keep"), "keep me").unwrap();
        let run = |path, remove| {
            codesync::platform::command("sh")
                .args([
                    "-c",
                    &script(&codesync::platform::local_path(&scratch.0), path, remove).unwrap(),
                ])
                .output()
                .unwrap()
        };
        assert!(run(name, false).status.success());
        assert!(scratch.0.join(name).exists());
        assert!(run(name, true).status.success());
        assert!(!scratch.0.join(name).exists());
        assert_eq!(
            String::from_utf8(run(name, true).stdout).unwrap(),
            "missing\n"
        );
        assert!(!run("sub", true).status.success());
        assert_eq!(
            std::fs::read_to_string(scratch.0.join("keep")).unwrap(),
            "keep me"
        );
        for path in [
            "",
            "/keep",
            "../keep",
            "sub/../../keep",
            "sub//file",
            ".",
            "sub/./file",
            "bad\0name",
        ] {
            assert!(script("/root", path, true).is_err());
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("keep", scratch.0.join("link")).unwrap();
            assert!(!run("link", true).status.success());
            std::os::unix::fs::symlink(&scratch.0, scratch.0.join("escape")).unwrap();
            assert!(!run("escape/keep", true).status.success());
            assert!(scratch.0.join("keep").exists());
        }
    }
}
