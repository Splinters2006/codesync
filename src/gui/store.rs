use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn remote_directory(input: &str, local: &Path) -> Result<String, String> {
    let trimmed = input.trim().trim_end_matches('/');
    let base = if trimmed.is_empty() && input.trim().is_empty() || trimmed == "~/codesync" {
        "codesync"
    } else {
        trimmed
    };
    if base == "codesync" || (base.starts_with('/') && base.ends_with("/codesync")) {
        let name = local.file_name().and_then(|name| name.to_str()).filter(|name| !name.is_empty()).ok_or("Choose a local folder with a valid directory name, or enter an explicit remote path.")?;
        Ok(format!("{base}/{name}"))
    } else {
        Ok(input.trim().into())
    }
}

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub enum ConnectionMode {
    #[default]
    Automatic,
    LocalOnly,
    RemoteOnly,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkSettings {
    pub mode: ConnectionMode,
    pub public_host: String,
    pub public_port: u16,
    pub tailscale_host: String,
    pub tailscale_port: u16,
}
impl Default for NetworkSettings {
    fn default() -> Self {
        Self {
            mode: ConnectionMode::Automatic,
            public_host: String::new(),
            public_port: 22,
            tailscale_host: String::new(),
            tailscale_port: 22,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Server {
    pub id: u64,
    pub name: String,
    pub host: String,
    pub user: String,
    pub port: u16,
    #[serde(default)]
    pub network: NetworkSettings,
}
impl Server {
    pub fn identity_alias(&self) -> String {
        format!("codesync-server-{}", self.id)
    }
    pub fn endpoints(&self) -> Vec<(&str, u16, &'static str)> {
        let mut endpoints = Vec::new();
        if self.network.mode != ConnectionMode::LocalOnly && !self.network.tailscale_host.is_empty()
        {
            endpoints.push((
                self.network.tailscale_host.as_str(),
                self.network.tailscale_port,
                "Tailscale",
            ));
        }
        if self.network.mode != ConnectionMode::RemoteOnly {
            endpoints.push((self.host.as_str(), self.port, "Local / primary"));
        }
        if self.network.mode != ConnectionMode::LocalOnly && !self.network.public_host.is_empty() {
            endpoints.push((
                self.network.public_host.as_str(),
                self.network.public_port,
                "Public",
            ));
        }
        endpoints
    }
    pub fn destination(&self) -> String {
        let destination = if self.user.is_empty() {
            self.host.clone()
        } else {
            format!("{}@{}", self.user, self.host)
        };
        codesync::ssh_destination(&destination)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() || self.host.trim().is_empty() || self.port == 0 {
            return Err("Enter a server name, address, and a port between 1 and 65535.".into());
        }
        if self.host.contains('@') || self.user.contains('@') {
            return Err("Enter the IP / hostname and username in their separate fields.".into());
        }
        for (host, port) in [
            (&self.network.public_host, self.network.public_port),
            (&self.network.tailscale_host, self.network.tailscale_port),
        ] {
            if !host.is_empty() {
                if port == 0 || host.contains('@') {
                    return Err("Enter the remote address and port separately.".into());
                }
                codesync::Config {
                    host: if self.user.is_empty() {
                        host.clone()
                    } else {
                        format!("{}@{host}", self.user)
                    },
                    dir: "/work".into(),
                }
                .validate()?;
            }
        }
        codesync::Config {
            host: if self.user.is_empty() {
                self.host.clone()
            } else {
                format!("{}@{}", self.user, self.host)
            },
            dir: "/work".into(),
        }
        .validate()
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Folder {
    pub id: u64,
    pub name: String,
    pub path: PathBuf,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Link {
    pub id: u64,
    pub folder: u64,
    pub server: u64,
    pub remote: String,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Data {
    pub servers: Vec<Server>,
    pub folders: Vec<Folder>,
    pub links: Vec<Link>,
}
impl Data {
    /// Build a complete batch before saving anything or starting transfers.
    pub fn share_links(
        &self,
        folders: &std::collections::HashSet<u64>,
        servers: &std::collections::HashSet<u64>,
    ) -> Result<(Self, Vec<Link>), String> {
        if folders.is_empty() || servers.is_empty() {
            return Err("Select at least one folder and one server.".into());
        }
        let mut data = self.clone();
        let mut links = Vec::new();
        for folder in self.folders.iter().filter(|f| folders.contains(&f.id)) {
            for server in self.servers.iter().filter(|s| servers.contains(&s.id)) {
                if let Some(link) = self
                    .links
                    .iter()
                    .find(|l| l.folder == folder.id && l.server == server.id)
                {
                    links.push(link.clone());
                    continue;
                }
                let link = Link {
                    id: data.next_id(),
                    folder: folder.id,
                    server: server.id,
                    remote: remote_directory("codesync", &folder.path)?,
                };
                data.validate_link(&link).map_err(|e| format!("Cannot link {} to {}: {e} Open the folder and add a link with a distinct remote directory.", folder.name, server.name))?;
                data.links.push(link.clone());
                links.push(link);
            }
        }
        if links.len() != folders.len() * servers.len() {
            return Err("A selected folder or server no longer exists. Select again.".into());
        }
        Ok((data, links))
    }
    pub fn next_id(&self) -> u64 {
        let maximum = self
            .servers
            .iter()
            .map(|s| s.id)
            .chain(self.folders.iter().map(|f| f.id))
            .chain(self.links.iter().map(|l| l.id))
            .max()
            .unwrap_or(0)
            + 1;
        // Never reuse a removed server's SSH identity alias.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_micros() as u64);
        maximum.max(now)
    }
    pub fn validate_link(&self, link: &Link) -> Result<(), String> {
        let folder = self
            .folders
            .iter()
            .find(|f| f.id == link.folder)
            .ok_or("Choose a local folder.")?;
        let remote = remote_directory(&link.remote, &folder.path)?;
        let server = self
            .servers
            .iter()
            .find(|s| s.id == link.server)
            .ok_or("Choose a server.")?;
        codesync::Config {
            host: server.destination(),
            dir: remote.clone(),
        }
        .validate()?;
        if self
            .links
            .iter()
            .any(|l| l.id != link.id && l.folder == link.folder && l.server == link.server)
        {
            return Err(
                "This folder is already linked to that server. Edit the existing link.".into(),
            );
        }
        if self.links.iter().any(|l| {
            l.id != link.id
                && l.server == link.server
                && self
                    .folders
                    .iter()
                    .find(|f| f.id == l.folder)
                    .and_then(|f| remote_directory(&l.remote, &f.path).ok())
                    .is_some_and(|existing| {
                        existing.trim_end_matches('/') == remote.trim_end_matches('/')
                    })
        }) {
            return Err("Another folder already uses this destination on that server.".into());
        }
        Ok(())
    }
    pub fn import_folder(&mut self, name: String, path: PathBuf) -> u64 {
        if let Some(folder) = self.folders.iter().find(|f| f.path == path) {
            return folder.id;
        }
        let folder = self.next_id();
        self.folders.push(Folder {
            id: folder,
            name,
            path: path.clone(),
        });
        if let Ok(config) = codesync::Config::read_at(&path) {
            let (user, host) = config.host.rsplit_once('@').unwrap_or(("", &config.host));
            let server = if let Some(server) = self
                .servers
                .iter()
                .find(|s| s.host == host && s.user == user && s.port == 22)
            {
                server.id
            } else {
                let id = self.next_id();
                self.servers.push(Server {
                    id,
                    name: host.into(),
                    host: host.into(),
                    user: user.into(),
                    port: 22,
                    network: Default::default(),
                });
                id
            };
            let link = Link {
                id: self.next_id(),
                folder,
                server,
                remote: config.dir,
            };
            if self.validate_link(&link).is_ok() {
                self.links.push(link);
            }
        }
        folder
    }
}
fn directory() -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .ok_or("Cannot locate your configuration directory.")?;
    Ok(base.join("codesync"))
}
pub fn load() -> Result<Data, String> {
    let dir = directory()?;
    match fs::read(dir.join("profiles.json")) {
        Ok(bytes) => {
            return serde_json::from_slice(&bytes)
                .map_err(|e| format!("Cannot read saved profiles: {e}"));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    #[derive(Deserialize)]
    struct OldFolder {
        name: String,
        path: PathBuf,
    }
    let mut data = Data::default();
    match fs::read(dir.join("folders.json")) {
        Ok(bytes) => {
            let old: Vec<OldFolder> = serde_json::from_slice(&bytes)
                .map_err(|e| format!("Cannot migrate folder list: {e}"))?;
            for f in old {
                data.import_folder(f.name, f.path);
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    if data.folders.is_empty()
        && let Ok(path) = std::env::current_dir()
        && path.join(".codesync").is_file()
    {
        data.import_folder(
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            path,
        );
    }
    if !data.folders.is_empty() || !data.servers.is_empty() {
        // Persist migrated IDs before any SSH identity is enrolled against them.
        save_at(&data, &dir.join("profiles.json"))?;
    }
    Ok(data)
}
pub fn save(data: &Data) -> Result<(), String> {
    save_at(data, &directory()?.join("profiles.json"))
}
fn save_at(data: &Data, path: &Path) -> Result<(), String> {
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(
        &temp,
        serde_json::to_vec_pretty(data).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::rename(temp, path).map_err(|e| e.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn home_share_builds_all_pairs_preserves_links_and_rejects_collisions() {
        let mut data = Data::default();
        for id in 1..=3 {
            data.servers.push(Server {
                id,
                name: format!("Server {id}"),
                host: format!("server{id}"),
                user: "user".into(),
                port: 22,
                network: Default::default(),
            });
        }
        for id in 4..=5 {
            data.folders.push(Folder {
                id,
                name: format!("Folder {id}"),
                path: format!("/tmp/folder{id}").into(),
            });
        }
        data.links.push(Link {
            id: 6,
            folder: 4,
            server: 1,
            remote: "/srv/existing".into(),
        });
        let folders = [4, 5].into_iter().collect();
        let servers = [1, 2, 3].into_iter().collect();
        let (shared, links) = data.share_links(&folders, &servers).unwrap();
        assert_eq!(links.len(), 6);
        assert_eq!(shared.links.len(), 6);
        assert_eq!(links[0].remote, "/srv/existing");
        assert_eq!(links[1].remote, "codesync/folder4");
        assert_eq!(
            links
                .iter()
                .map(|l| l.id)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            6
        );
        assert_eq!(
            shared
                .share_links(&folders, &servers)
                .unwrap()
                .0
                .links
                .len(),
            6
        );
        assert_eq!(data.links.len(), 1);
        data.folders[1].path = "/other/folder4".into();
        assert!(data.share_links(&folders, &servers).is_err());
        assert_eq!(data.links.len(), 1);
        assert!(
            data.share_links(&[99].into_iter().collect(), &servers)
                .is_err()
        );
    }

    #[test]
    fn codesync_parent_keeps_the_local_directory_name() {
        let local = Path::new("/work/test4");
        for input in ["", "   ", "codesync", "codesync/", "~/codesync"] {
            let dir = remote_directory(input, local).unwrap();
            assert_eq!(dir, "codesync/test4");
            assert!(
                codesync::Config {
                    host: "user@server".into(),
                    dir
                }
                .validate()
                .is_ok()
            );
        }
        assert_eq!(
            remote_directory("/srv/codesync/", local).unwrap(),
            "/srv/codesync/test4"
        );
        assert_eq!(
            remote_directory("codesync/test4", local).unwrap(),
            "codesync/test4"
        );
        assert_eq!(
            remote_directory(" /srv/notes ", local).unwrap(),
            "/srv/notes"
        );
        assert_eq!(remote_directory("/", local).unwrap(), "/");
        assert!(remote_directory("codesync", Path::new("/")).is_err());
        assert_eq!(
            codesync::rsync_destination("user@server", &remote_directory("", local).unwrap()),
            "user@server:codesync/test4/"
        );
    }

    #[test]
    fn old_profiles_migrate_and_connection_modes_restrict_fallback() {
        let mut server: Server = serde_json::from_str(
            r#"{"id":1,"name":"Home","host":"192.168.1.10","user":"user","port":22}"#,
        )
        .unwrap();
        assert_eq!(
            server.endpoints(),
            vec![("192.168.1.10", 22, "Local / primary")]
        );
        server.network.public_host = "home.example.com".into();
        server.network.public_port = 2222;
        server.network.tailscale_host = "100.100.1.2".into();
        assert_eq!(
            server.endpoints(),
            vec![
                ("100.100.1.2", 22, "Tailscale"),
                ("192.168.1.10", 22, "Local / primary"),
                ("home.example.com", 2222, "Public")
            ]
        );
        server.network.mode = ConnectionMode::RemoteOnly;
        assert_eq!(server.endpoints().len(), 2);
        assert!(
            server
                .endpoints()
                .iter()
                .all(|(host, _, _)| *host != server.host)
        );
        server.network.mode = ConnectionMode::LocalOnly;
        assert_eq!(
            server.endpoints(),
            vec![("192.168.1.10", 22, "Local / primary")]
        );
    }
    #[test]
    fn supports_many_to_many_links_and_roundtrip() {
        let mut data = Data::default();
        for id in 1..=3 {
            data.servers.push(Server {
                id,
                name: format!("Server {id}"),
                host: format!("server{id}"),
                user: "user".into(),
                port: 22,
                network: Default::default(),
            });
        }
        for id in 4..=6 {
            data.folders.push(Folder {
                id,
                name: format!("Folder {id}"),
                path: format!("/tmp/folder{id}").into(),
            });
        }
        for (folder, server) in [(4, 1), (4, 2), (4, 3), (5, 1), (6, 1)] {
            let link = Link {
                id: data.next_id(),
                folder,
                server,
                remote: format!("/home/user/folder{folder}"),
            };
            data.validate_link(&link).unwrap();
            data.links.push(link);
        }
        let bytes = serde_json::to_vec(&data).unwrap();
        let restored: Data = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored.links.iter().filter(|l| l.folder == 4).count(), 3);
        assert_eq!(restored.links.iter().filter(|l| l.server == 1).count(), 3);
        let duplicate = Link {
            id: 100,
            folder: 5,
            server: 2,
            remote: "/home/user/folder4/".into(),
        };
        assert!(restored.validate_link(&duplicate).is_err());
        assert!(!String::from_utf8(bytes).unwrap().contains("password"));
    }
}
