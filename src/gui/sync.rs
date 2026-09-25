//! Content-based merge planning. Originals are renamed, never overwritten.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

pub struct Scratch(pub PathBuf);
impl Scratch {
    pub fn new() -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..100 {
            let path = std::env::temp_dir().join(format!(
                "codesync-merge-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
        Err("Cannot create sync workspace.".into())
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("Stopped. Completed copies and conflict renames remain in place.".into())
    } else {
        Ok(())
    }
}
pub fn hash(path: &Path) -> Result<String, String> {
    let out = Command::new("sha256sum")
        .arg("--")
        .arg(path)
        .output()
        .map_err(|e| format!("SHA-256 requires sha256sum: {e}"))?;
    if !out.status.success() {
        return Err("Cannot hash a file. Check permissions and retry sync.".into());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    // GNU sha256sum prefixes escaped filenames with a backslash.
    let text = text.strip_prefix('\\').unwrap_or(&text);
    let digest: String = text.chars().take(64).collect();
    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid SHA-256 result.".into());
    }
    Ok(digest)
}
#[derive(Default)]
pub struct Tree {
    pub files: BTreeMap<PathBuf, String>,
    pub dirs: BTreeSet<PathBuf>,
}
pub fn scan(root: &Path, cancel: &AtomicBool) -> Result<Tree, String> {
    fn walk(
        root: &Path,
        relative: &Path,
        tree: &mut Tree,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        for item in fs::read_dir(root.join(relative)).map_err(|e| e.to_string())? {
            check_cancel(cancel)?;
            let item = item.map_err(|e| e.to_string())?;
            let path = relative.join(item.file_name());
            let kind = item.file_type().map_err(|e| e.to_string())?;
            if kind.is_dir() {
                tree.dirs.insert(path.clone());
                walk(root, &path, tree, cancel)?;
            } else if kind.is_file() {
                tree.files.insert(path.clone(), hash(&root.join(path))?);
            } else {
                return Err("Two-way sync supports regular files and directories only. Remove symbolic links or special files from the shared folder.".into());
            }
        }
        Ok(())
    }
    let mut tree = Tree::default();
    walk(root, Path::new(""), &mut tree, cancel)?;
    Ok(tree)
}
pub struct Rename {
    pub participant: usize,
    pub from: PathBuf,
    pub to: PathBuf,
    pub hash: String,
}
pub struct Plan {
    pub renames: Vec<Rename>,
    pub files: BTreeMap<PathBuf, (usize, PathBuf)>,
    pub dirs: BTreeSet<PathBuf>,
}
fn label(value: &str) -> String {
    let label: String = value
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || "-_".contains(c) {
                c
            } else {
                '_'
            }
        })
        .take(40)
        .collect();
    if label.is_empty() {
        "Host".into()
    } else {
        label
    }
}
pub fn plan(trees: &[Tree], names: &[String]) -> Result<Plan, String> {
    if trees.len() != names.len() {
        return Err("Invalid sync participants.".into());
    }
    let mut dirs = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for tree in trees {
        dirs.extend(tree.dirs.iter().cloned());
        paths.extend(tree.files.keys().cloned());
    }
    if paths.iter().any(|p| dirs.contains(p)) {
        return Err("A file and a directory use the same path on different hosts. Rename one before syncing.".into());
    }
    let mut reserved = paths.clone();
    reserved.extend(dirs.iter().cloned());
    let mut result = Plan {
        renames: Vec::new(),
        files: BTreeMap::new(),
        dirs,
    };
    for path in paths {
        let mut versions: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (index, tree) in trees.iter().enumerate() {
            if let Some(hash) = tree.files.get(&path) {
                versions.entry(hash).or_default().push(index);
            }
        }
        if versions.len() == 1 {
            let source = versions.values().next().unwrap()[0];
            result.files.insert(path.clone(), (source, path));
            continue;
        }
        for (digest, owners) in versions {
            let source = owners[0];
            let mut filename = std::ffi::OsString::from(format!("{}_", label(&names[source])));
            filename.push(path.file_name().unwrap());
            let mut target = path.with_file_name(&filename);
            let mut suffix = 0;
            while reserved.contains(&target) {
                suffix += 1;
                let mut numbered = std::ffi::OsString::from(format!(
                    "{}_{}_{}_",
                    label(&names[source]),
                    &digest[..8],
                    suffix
                ));
                numbered.push(path.file_name().unwrap());
                target = path.with_file_name(numbered);
            }
            reserved.insert(target.clone());
            result.files.insert(target.clone(), (source, path.clone()));
            for participant in owners {
                result.renames.push(Rename {
                    participant,
                    from: path.clone(),
                    to: target.clone(),
                    hash: digest.into(),
                });
            }
        }
    }
    Ok(result)
}
pub fn materialize(
    plan: &Plan,
    snapshots: &[PathBuf],
    output: &Path,
    cancel: &AtomicBool,
) -> Result<(), String> {
    fs::create_dir_all(output).map_err(|e| e.to_string())?;
    for dir in &plan.dirs {
        fs::create_dir_all(output.join(dir)).map_err(|e| e.to_string())?;
    }
    for (target, (source, path)) in &plan.files {
        check_cancel(cancel)?;
        let destination = output.join(target);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::copy(snapshots[*source].join(path), destination).map_err(|e| e.to_string())?;
    }
    Ok(())
}
pub fn local_rename(root: &Path, rename: &Rename) -> Result<(), String> {
    let from = root.join(&rename.from);
    let to = root.join(&rename.to);
    for ancestor in rename
        .from
        .ancestors()
        .filter(|p| !p.as_os_str().is_empty())
    {
        if fs::symlink_metadata(root.join(ancestor))
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
        {
            return Err("Folder changed during sync. Retry after edits finish.".into());
        }
    }
    if hash(&from)? != rename.hash {
        return Err("A file changed during sync. Retry after edits finish.".into());
    }
    use std::os::unix::ffi::OsStrExt;
    let from = std::ffi::CString::new(from.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let to = std::ffi::CString::new(to.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result != 0 {
        return Err(format!(
            "Cannot preserve conflict without overwriting a file: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tree(root: &Path, cancel: &AtomicBool) -> Tree {
        scan(root, cancel).unwrap()
    }
    #[test]
    fn three_hosts_keep_every_version_and_repeat_without_new_conflicts() {
        let scratch = Scratch::new().unwrap();
        let roots: Vec<_> = (0..3).map(|i| scratch.0.join(i.to_string())).collect();
        let cancel = AtomicBool::new(false);
        for (i, root) in roots.iter().enumerate() {
            fs::create_dir_all(root.join("nested")).unwrap();
            fs::write(root.join("nested/file.txt"), format!("version {i}")).unwrap();
            fs::write(root.join(format!("unique-{i}")), format!("unique {i}")).unwrap();
            fs::write(root.join("same"), "identical content").unwrap();
        }
        fs::write(
            roots[0].join("nested/Host_file.txt"),
            "existing prefix must survive",
        )
        .unwrap();
        let names = vec!["Host".into(), "Server".into(), "Server".into()];
        let trees: Vec<_> = roots.iter().map(|root| tree(root, &cancel)).collect();
        let plan = plan(&trees, &names).unwrap();
        assert_eq!(plan.renames.len(), 3);
        assert_eq!(plan.files.len(), 8);
        assert!(
            plan.renames
                .iter()
                .all(|r| r.to != Path::new("nested/Host_file.txt"))
        );
        let merged = scratch.0.join("merged");
        materialize(&plan, &roots, &merged, &cancel).unwrap();
        for rename in &plan.renames {
            local_rename(&roots[rename.participant], rename).unwrap();
        }
        for root in &roots {
            let status = Command::new("rsync")
                .args(["-a", "--ignore-existing", "--"])
                .arg(format!("{}/", merged.display()))
                .arg(root)
                .status()
                .unwrap();
            assert!(status.success());
            assert!(!root.join("nested/file.txt").exists());
            assert_eq!(tree(root, &cancel).files, tree(&merged, &cancel).files);
        }
        let trees: Vec<_> = roots.iter().map(|root| tree(root, &cancel)).collect();
        let repeated = super::plan(&trees, &names).unwrap();
        assert!(repeated.renames.is_empty());
        assert_eq!(repeated.files.len(), 8);
    }
    #[test]
    fn refuses_changed_files_collisions_and_symlinks() {
        let scratch = Scratch::new().unwrap();
        let cancel = AtomicBool::new(false);
        fs::write(scratch.0.join("file"), "original").unwrap();
        let rename = Rename {
            participant: 0,
            from: "file".into(),
            to: "Host_file".into(),
            hash: hash(&scratch.0.join("file")).unwrap(),
        };
        fs::write(scratch.0.join("file"), "new edit").unwrap();
        assert!(local_rename(&scratch.0, &rename).is_err());
        assert_eq!(
            fs::read_to_string(scratch.0.join("file")).unwrap(),
            "new edit"
        );
        fs::write(scratch.0.join("file"), "original").unwrap();
        fs::write(scratch.0.join("Host_file"), "keep").unwrap();
        assert!(local_rename(&scratch.0, &rename).is_err());
        assert_eq!(
            fs::read_to_string(scratch.0.join("Host_file")).unwrap(),
            "keep"
        );
        std::os::unix::fs::symlink("file", scratch.0.join("link")).unwrap();
        assert!(scan(&scratch.0, &cancel).is_err());
        assert!(scan(&scratch.0, &AtomicBool::new(true)).is_err());
        let mut file = Tree::default();
        file.files.insert("path".into(), "a".repeat(64));
        let mut dir = Tree::default();
        dir.dirs.insert("path".into());
        assert!(plan(&[file, dir], &["A".into(), "B".into()]).is_err());
    }
}
