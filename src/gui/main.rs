mod auth;
mod jobs;
mod store;
mod sync;

use auth::Credentials;
use eframe::egui::{self, Color32, RichText};
use jobs::{Action, Task};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::mpsc,
};
use store::{ConnectionMode, Data, Link, Server};

fn main() -> eframe::Result {
    if std::env::var_os("CODESYNC_ASKPASS").is_some() {
        std::process::exit(if auth::askpass().is_ok() { 0 } else { 1 });
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Codesync - File Synchronization")
            .with_inner_size([1100.0, 760.0])
            .with_min_inner_size([820.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Codesync",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    Folder(u64),
    Server(u64),
}
struct ServerEditor {
    server: Server,
    credentials: Credentials,
    error: Option<String>,
}
struct LinkEditor {
    link: Link,
    error: Option<String>,
}
struct App {
    data: Data,
    home: bool,
    connections: bool,
    host_password_purpose: String,
    home_folders: HashSet<u64>,
    home_servers: HashSet<u64>,
    credentials: HashMap<u64, Credentials>,
    selected: Option<Selection>,
    selected_links: HashSet<u64>,
    server_editor: Option<ServerEditor>,
    link_editor: Option<LinkEditor>,
    new_folder: bool,
    folder_name: String,
    folder_path: String,
    picker: Option<mpsc::Receiver<Result<String, String>>>,
    job: Option<jobs::Job>,
    trust: Option<(String, mpsc::Sender<bool>)>,
    host_password: Option<mpsc::Sender<Option<String>>>,
    host_password_input: String,
    logins: Vec<(String, String)>,
    output: jobs::Output,
    status: String,
    error: Option<String>,
}
fn classic_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::light();
    visuals.panel_fill = Color32::from_gray(224);
    visuals.window_fill = Color32::from_gray(236);
    visuals.extreme_bg_color = Color32::WHITE;
    visuals.faint_bg_color = Color32::from_gray(245);
    visuals.override_text_color = Some(Color32::from_gray(25));
    visuals.selection.bg_fill = Color32::from_gray(185);
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, Color32::BLACK);
    visuals.window_corner_radius = egui::CornerRadius::ZERO;
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = egui::CornerRadius::ZERO;
        widget.bg_stroke = egui::Stroke::new(1.0_f32, Color32::from_gray(130));
    }
    visuals.widgets.inactive.bg_fill = Color32::from_gray(232);
    visuals.widgets.hovered.bg_fill = Color32::from_gray(250);
    visuals.widgets.active.bg_fill = Color32::from_gray(190);
    ctx.set_visuals(visuals);
    ctx.style_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 5.0);
    });
}
impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        classic_style(&cc.egui_ctx);
        let (data, error) = match store::load() {
            Ok(data) => (data, None),
            Err(e) => (Data::default(), Some(e)),
        };
        let selected = data
            .folders
            .first()
            .map(|f| Selection::Folder(f.id))
            .or_else(|| data.servers.first().map(|s| Selection::Server(s.id)));
        Self {
            data,
            home: true,
            connections: false,
            host_password_purpose: String::new(),
            home_folders: HashSet::new(),
            home_servers: HashSet::new(),
            credentials: HashMap::new(),
            selected,
            selected_links: HashSet::new(),
            server_editor: None,
            link_editor: None,
            new_folder: false,
            folder_name: String::new(),
            folder_path: String::new(),
            picker: None,
            job: None,
            trust: None,
            host_password: None,
            host_password_input: String::new(),
            logins: Vec::new(),
            output: jobs::Output::default(),
            status: "Ready".into(),
            error,
        }
    }
    fn persist(&mut self) {
        if let Err(e) = store::save(&self.data) {
            self.error = Some(e);
        }
    }
    fn edit_server(&mut self, id: Option<u64>) {
        let server = id
            .and_then(|id| self.data.servers.iter().find(|s| s.id == id).cloned())
            .unwrap_or(Server {
                id: self.data.next_id(),
                name: String::new(),
                host: String::new(),
                user: String::new(),
                port: 22,
                network: Default::default(),
            });
        let credentials = self
            .credentials
            .get(&server.id)
            .cloned()
            .unwrap_or(Credentials {
                same_password: true,
                ..Default::default()
            });
        self.server_editor = Some(ServerEditor {
            server,
            credentials,
            error: None,
        });
    }
    fn new_link(&mut self) {
        let folder = match self.selected {
            Some(Selection::Folder(id)) => id,
            _ => self.data.folders.first().map_or(0, |f| f.id),
        };
        let server = match self.selected {
            Some(Selection::Server(id)) => id,
            _ => self.data.servers.first().map_or(0, |s| s.id),
        };
        self.link_editor = Some(LinkEditor {
            link: Link {
                id: self.data.next_id(),
                folder,
                server,
                remote: String::new(),
            },
            error: None,
        });
    }
    fn add_folder(&mut self) {
        let path = if let Some(rest) = self.folder_path.trim().strip_prefix("~/") {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(rest))
                .unwrap_or_else(|| self.folder_path.trim().into())
        } else {
            self.folder_path.trim().into()
        };
        let Ok(path) = path.canonicalize() else {
            self.error = Some("Choose an existing local directory.".into());
            return;
        };
        if !path.is_dir() {
            self.error = Some("The local path must be a directory.".into());
            return;
        }
        let name = if self.folder_name.trim().is_empty() {
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        } else {
            self.folder_name.trim().into()
        };
        let id = self.data.import_folder(name, path);
        self.selected = Some(Selection::Folder(id));
        self.selected_links.clear();
        self.persist();
        self.new_folder = false;
        self.folder_name.clear();
        self.folder_path.clear();
    }
    fn browse(&mut self, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = std::process::Command::new("zenity")
                .args([
                    "--file-selection",
                    "--directory",
                    "--title=Choose local folder",
                ])
                .output()
                .map_err(|e| format!("Folder picker unavailable ({e}). Enter the path manually."))
                .map(|out| {
                    if out.status.success() {
                        String::from_utf8_lossy(&out.stdout).trim().into()
                    } else {
                        String::new()
                    }
                });
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.picker = Some(rx);
    }
    fn task(&self, link: &Link, action: Action) -> Option<Task> {
        let server = self
            .data
            .servers
            .iter()
            .find(|s| s.id == link.server)?
            .clone();
        let local = self
            .data
            .folders
            .iter()
            .find(|f| f.id == link.folder)?
            .path
            .clone();
        Some(Task {
            credentials: self
                .credentials
                .get(&server.id)
                .cloned()
                .unwrap_or_default(),
            server,
            local,
            remote: store::remote_directory(&link.remote),
            action,
        })
    }
    fn start(&mut self, tasks: Vec<Task>, title: &str, ctx: &egui::Context) {
        if tasks.is_empty() {
            self.error = Some("Add a server link first.".into());
            return;
        }
        self.error = None;
        self.logins.clear();
        self.status = title.into();
        self.output = jobs::Output::default();
        self.output.append(format!("{title}\n").as_bytes());
        self.job = Some(jobs::Job::start(tasks, ctx.clone()));
    }
    fn server_action(&mut self, id: u64, action: Action, ctx: &egui::Context) {
        if let Some(server) = self.data.servers.iter().find(|s| s.id == id).cloned() {
            let title = match action {
                Action::Setup => "Setting up server",
                Action::Tailscale => "Setting up Tailscale",
                _ => "Testing connection",
            };
            let task = Task {
                credentials: self.credentials.get(&id).cloned().unwrap_or_default(),
                server,
                local: std::env::temp_dir(),
                remote: String::new(),
                action,
            };
            self.start(vec![task], title, ctx);
        }
    }
    fn poll(&mut self) {
        let mut finished = None;
        let mut tailscale_ready = None;
        if let Some(job) = &self.job {
            for event in job.events.try_iter().take(64) {
                match event {
                    jobs::Event::Output(bytes) => self.output.append(&bytes),
                    jobs::Event::TrustHost { prompt, response } => {
                        self.trust = Some((prompt, response))
                    }
                    jobs::Event::HostPassword { response, purpose } => {
                        self.host_password_purpose = purpose;
                        self.host_password_input.clear();
                        self.host_password = Some(response);
                    }
                    jobs::Event::Login { label, url } => {
                        self.logins.retain(|(existing, _)| *existing != label);
                        self.logins.push((label, url));
                    }
                    jobs::Event::TailscaleReady { server, host, port } => {
                        tailscale_ready = Some((server, host, port))
                    }
                    jobs::Event::Finished(result) => finished = Some(result),
                }
            }
        }
        if let Some((id, host, port)) = tailscale_ready
            && let Some(server) = self.data.servers.iter_mut().find(|s| s.id == id)
        {
            server.network.tailscale_host = host;
            server.network.tailscale_port = port;
            self.persist();
            self.output.append(b"Tailscale address saved after identity verification. Automatic or Remote only mode can now use it.\n");
        }
        if let Some(result) = finished {
            self.logins.clear();
            self.job = None;
            self.trust = None;
            self.host_password = None;
            self.host_password_input.clear();
            match result {
                Ok(()) => self.status = "Completed successfully".into(),
                Err(e) => {
                    self.status = "Operation failed or stopped".into();
                    self.error = Some(e);
                }
            }
        }
        if let Some(picker) = &self.picker
            && let Ok(result) = picker.try_recv()
        {
            self.picker = None;
            match result {
                Ok(path) if !path.is_empty() => self.folder_path = path,
                Err(e) => self.error = Some(e),
                _ => {}
            }
        }
    }
    fn visible_links(&self) -> Vec<Link> {
        self.data
            .links
            .iter()
            .filter(|l| match self.selected {
                Some(Selection::Folder(id)) => l.folder == id,
                Some(Selection::Server(id)) => l.server == id,
                None => false,
            })
            .cloned()
            .collect()
    }
    fn show_home(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let busy = self.job.is_some();
        self.home_folders
            .retain(|id| self.data.folders.iter().any(|f| f.id == *id));
        self.home_servers
            .retain(|id| self.data.servers.iter().any(|s| s.id == *id));
        ui.heading("Home");
        ui.label("Choose folders and servers to share. Each selected folder syncs with every selected server.");
        let mut open_folder = None;
        let mut open_server = None;
        let mut add_server = false;
        ui.add_enabled_ui(!busy, |ui| {
            ui.columns(2, |columns| {
                columns[0].heading("Folders");
                columns[0].separator();
                ui_home_select_all(
                    &mut columns[0],
                    &mut self.home_folders,
                    self.data.folders.iter().map(|f| f.id),
                );
                egui::ScrollArea::vertical()
                    .id_salt("home-folders")
                    .max_height(300.0)
                    .show(&mut columns[0], |ui| {
                        for folder in &self.data.folders {
                            ui.horizontal(|ui| {
                                let mut checked = self.home_folders.contains(&folder.id);
                                if ui.checkbox(&mut checked, &folder.name).changed() {
                                    if checked {
                                        self.home_folders.insert(folder.id);
                                    } else {
                                        self.home_folders.remove(&folder.id);
                                    }
                                }
                                if ui.small_button("Open").clicked() {
                                    open_folder = Some(folder.id);
                                }
                            });
                        }
                        if self.data.folders.is_empty() {
                            ui.label("No folders added");
                        }
                    });
                if columns[0].button("Add folder...").clicked() {
                    self.new_folder = true;
                }
                columns[1].heading("Servers");
                columns[1].separator();
                ui_home_select_all(
                    &mut columns[1],
                    &mut self.home_servers,
                    self.data.servers.iter().map(|s| s.id),
                );
                egui::ScrollArea::vertical()
                    .id_salt("home-servers")
                    .max_height(300.0)
                    .show(&mut columns[1], |ui| {
                        for server in &self.data.servers {
                            ui.horizontal(|ui| {
                                let mut checked = self.home_servers.contains(&server.id);
                                if ui.checkbox(&mut checked, &server.name).changed() {
                                    if checked {
                                        self.home_servers.insert(server.id);
                                    } else {
                                        self.home_servers.remove(&server.id);
                                    }
                                }
                                if ui.small_button("Open").clicked() {
                                    open_server = Some(server.id);
                                }
                            });
                        }
                        if self.data.servers.is_empty() {
                            ui.label("No servers added");
                        }
                    });
                if columns[1].button("Add server...").clicked() {
                    add_server = true;
                }
            });
        });
        if add_server {
            self.edit_server(None);
        }
        if let Some(id) = open_folder {
            self.selected = Some(Selection::Folder(id));
            self.home = false;
            self.selected_links.clear();
        }
        if let Some(id) = open_server {
            self.selected = Some(Selection::Server(id));
            self.home = false;
            self.selected_links.clear();
        }
        ui.separator();
        ui.label(format!(
            "{} folders × {} servers = {} transfers",
            self.home_folders.len(),
            self.home_servers.len(),
            self.home_folders.len() * self.home_servers.len()
        ));
        ui.label("Existing links keep their destination. New links use ~/codesync/<folder name> on each server, keeping folders separate.");
        let ready = !busy && !self.home_folders.is_empty() && !self.home_servers.is_empty();
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(ready, egui::Button::new("Sync")).clicked() {
                let result = self
                    .data
                    .share_links(&self.home_folders, &self.home_servers);
                match result {
                    Ok((data, links)) => match store::save(&data) {
                        Ok(()) => {
                            self.data = data;
                            let mut tasks = Vec::new();
                            for server in &self.data.servers {
                                if let Some(link) = links.iter().find(|l| l.server == server.id)
                                    && let Some(task) = self.task(link, Action::Test)
                                {
                                    tasks.push(task);
                                }
                            }
                            tasks.extend(
                                links
                                    .iter()
                                    .filter_map(|link| self.task(link, Action::Sync)),
                            );
                            self.start(tasks, "Syncing selected folders", ctx);
                        }
                        Err(e) => self.error = Some(e),
                    },
                    Err(e) => self.error = Some(e),
                }
            }
        });
        ui.label("Sync compares SHA-256 hashes. Different versions get host/server prefixes. Missing files are copied; deletions are not propagated.");
    }
    fn show_details(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let busy = self.job.is_some();
        match self.selected {
            Some(Selection::Folder(id)) => {
                if let Some(folder) = self.data.folders.iter().find(|f| f.id == id).cloned() {
                    ui.heading(&folder.name);
                    ui.label(folder.path.display().to_string());
                    ui.label(
                        "Link this folder to one or more servers. Check the server links to sync.",
                    );
                    if ui
                        .add_enabled(!busy, egui::Button::new("Remove folder from list"))
                        .on_hover_text("Removes its links. No local or remote files are deleted.")
                        .clicked()
                    {
                        self.data.folders.retain(|f| f.id != id);
                        self.data.links.retain(|l| l.folder != id);
                        self.selected = None;
                        self.selected_links.clear();
                        self.persist();
                        return;
                    }
                }
            }
            Some(Selection::Server(id)) => {
                if let Some(server) = self.data.servers.iter().find(|s| s.id == id).cloned() {
                    ui.heading(&server.name);
                    ui.label(match server.network.mode {
                        ConnectionMode::Automatic => {
                            "Connection: Automatic (Tailscale, local, then public)"
                        }
                        ConnectionMode::LocalOnly => "Connection: Local only",
                        ConnectionMode::RemoteOnly => {
                            "Connection: Remote only (Tailscale, then public)"
                        }
                    });
                    let ready = self
                        .credentials
                        .get(&id)
                        .is_some_and(|c| !c.login.is_empty());
                    ui.label(if ready {
                        "Login password available for this session."
                    } else {
                        "Using SSH keys / agent. Edit server to enter a login password."
                    });
                    ui.add_enabled_ui(!busy, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            if ui.button("Edit server / passwords").clicked() {
                                self.edit_server(Some(id));
                            }
                            if ui.button("Test connection").clicked() {
                                self.server_action(id, Action::Test, ctx);
                            }
                            if ui.button("Set up Tailscale").clicked() {
                                self.server_action(id, Action::Tailscale, ctx);
                            }
                            if ui.button("Set up server").clicked() {
                                self.server_action(id, Action::Setup, ctx);
                            }
                            if ui
                                .button("Remove server")
                                .on_hover_text("Removes its links. No remote files are deleted.")
                                .clicked()
                            {
                                self.data.servers.retain(|s| s.id != id);
                                self.data.links.retain(|l| l.server != id);
                                self.credentials.remove(&id);
                                self.selected = None;
                                self.selected_links.clear();
                                self.persist();
                            }
                        });
                    });
                }
            }
            None => {
                ui.heading("File Synchronization");
                ui.label("1. Add a server and enter its address and credentials.");
                ui.label("2. Add the local folders you want to sync.");
                ui.label("3. Link folders to servers and choose each remote directory.");
                return;
            }
        }
        ui.separator();
        let links = self.visible_links();
        self.selected_links
            .retain(|id| links.iter().any(|link| link.id == *id));
        ui.horizontal_wrapped(|ui| {
            ui.strong("Folder / server links");
            if ui
                .add_enabled(
                    !busy && !self.data.folders.is_empty() && !self.data.servers.is_empty(),
                    egui::Button::new("Add link..."),
                )
                .clicked()
            {
                self.new_link();
            }
            if ui
                .add_enabled(!busy && !links.is_empty(), egui::Button::new("Select all"))
                .clicked()
            {
                self.selected_links.extend(links.iter().map(|link| link.id));
            }
            if ui
                .add_enabled(
                    !busy && !self.selected_links.is_empty(),
                    egui::Button::new("Clear selection"),
                )
                .clicked()
            {
                self.selected_links.clear();
            }
            ui.label(format!("{} selected", self.selected_links.len()));
        });
        egui::Frame::new()
            .fill(Color32::WHITE)
            .stroke(egui::Stroke::new(1.0_f32, Color32::GRAY))
            .inner_margin(6.0)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                egui::ScrollArea::horizontal().show(ui, |ui| {
                    egui::Grid::new("links")
                        .striped(true)
                        .min_col_width(110.0)
                        .show(ui, |ui| {
                            ui.label("");
                            ui.strong("Local folder");
                            ui.strong("Server");
                            ui.strong("Remote folder");
                            ui.end_row();
                            for link in &links {
                                let folder = self
                                    .data
                                    .folders
                                    .iter()
                                    .find(|f| f.id == link.folder)
                                    .map_or("Missing folder", |f| &f.name);
                                let server = self
                                    .data
                                    .servers
                                    .iter()
                                    .find(|s| s.id == link.server)
                                    .map_or("Missing server", |s| &s.name);
                                let mut checked = self.selected_links.contains(&link.id);
                                if ui
                                    .add_enabled(!busy, egui::Checkbox::without_text(&mut checked))
                                    .changed()
                                {
                                    if checked {
                                        self.selected_links.insert(link.id);
                                    } else {
                                        self.selected_links.remove(&link.id);
                                    }
                                }
                                for label in [folder, server] {
                                    if ui
                                        .add_enabled(
                                            !busy,
                                            egui::Button::selectable(
                                                self.selected_links.contains(&link.id),
                                                label,
                                            ),
                                        )
                                        .clicked()
                                        && !self.selected_links.remove(&link.id)
                                    {
                                        self.selected_links.insert(link.id);
                                    }
                                }
                                ui.label(&link.remote);
                                ui.end_row();
                            }
                        });
                });
                if links.is_empty() {
                    ui.label("No links yet. Add a link to choose a server and remote directory.");
                }
            });
        let selected: Vec<_> = links
            .iter()
            .filter(|link| self.selected_links.contains(&link.id))
            .cloned()
            .collect();
        ui.add_enabled_ui(!busy && !selected.is_empty(), |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("Sync").clicked() {
                    let tasks = selected
                        .iter()
                        .filter_map(|link| self.task(link, Action::Sync))
                        .collect();
                    self.start(tasks, "Syncing selected folders", ctx);
                }
                if ui
                    .add_enabled(selected.len() == 1, egui::Button::new("Edit link"))
                    .clicked()
                {
                    self.link_editor = Some(LinkEditor {
                        link: selected[0].clone(),
                        error: None,
                    });
                }
                if ui
                    .button("Unlink selected")
                    .on_hover_text("No files are deleted")
                    .clicked()
                {
                    self.data
                        .links
                        .retain(|link| !self.selected_links.contains(&link.id));
                    self.selected_links.clear();
                    self.persist();
                }
            });
        });
        ui.label(
            RichText::new(
                "Sync keeps different versions with host/server prefixes. No deletions are propagated.",
            )
            .small(),
        );
    }
    fn show_dialogs(&mut self, ctx: &egui::Context) {
        if self.connections {
            let mut open = true;
            let mut connect = false;
            let mut prepare = false;
            egui::Window::new("Connections").open(&mut open).collapsible(false).resizable(false).default_width(460.0).show(ctx, |ui| {
                ui.label("Connect Linux hosts directly using SSH. A separate file server is optional.");
                ui.label("Both hosts must be online and reachable over your local network, Tailscale, or a configured public address.");
                ui.separator();
                ui.add_enabled_ui(self.job.is_none(), |ui| {
                    if ui.button("Connect another host...").clicked() { connect = true; }
                    ui.label("Enter its address and SSH account. Verify its fingerprint before syncing.");
                    if ui.button("Prepare this host to receive connections").clicked() { prepare = true; }
                    ui.label("Installs OpenSSH server, rsync, and SHA-256 tools, then enables SSH at startup. SSH grants access with that account's permissions, beyond the folders selected in Codesync. Existing SSH settings and firewall rules are preserved.");
                });
                ui.label("On the other host, add this host using its reachable address, SSH port, and account. Files sync when you click Sync; no pairing code or background sync is required.");
            });
            self.connections = open;
            if connect {
                self.connections = false;
                self.edit_server(None);
            }
            if prepare {
                self.connections = false;
                self.start(
                    vec![Task {
                        server: Server {
                            id: 0,
                            name: "This host".into(),
                            host: "localhost".into(),
                            user: String::new(),
                            port: 22,
                            network: Default::default(),
                        },
                        credentials: Credentials::default(),
                        local: std::env::temp_dir(),
                        remote: String::new(),
                        action: Action::PrepareHost,
                    }],
                    "Preparing this host for connections",
                    ctx,
                );
            }
        }

        if let Some(mut editor) = self.server_editor.take() {
            let mut open = true;
            let mut save = false;
            let mut setup = false;
            let mut tailscale = false;
            egui::Window::new("Server properties").open(&mut open).collapsible(false).resizable(false).default_width(470.0).vscroll(true).max_height(650.0).show(ctx, |ui| {
                field(ui, "Name", &mut editor.server.name, "Home server", false);
                field(ui, "Local / primary address", &mut editor.server.host, "192.168.1.10", false);
                ui.horizontal(|ui| { ui.label("Local / primary SSH port"); ui.add(egui::DragValue::new(&mut editor.server.port).range(1..=65535)); });
                field(ui, "Public address (optional)", &mut editor.server.network.public_host, "server.example.com", false);
                ui.horizontal(|ui| { ui.label("Public SSH port"); ui.add(egui::DragValue::new(&mut editor.server.network.public_port).range(1..=65535)); });
                ui.label("Connection choice");
                egui::ComboBox::from_id_salt("connection-mode").selected_text(match editor.server.network.mode {
                    ConnectionMode::Automatic => "Automatic", ConnectionMode::LocalOnly => "Local only", ConnectionMode::RemoteOnly => "Remote only",
                }).show_ui(ui, |ui| {
                    ui.selectable_value(&mut editor.server.network.mode, ConnectionMode::Automatic, "Automatic");
                    ui.selectable_value(&mut editor.server.network.mode, ConnectionMode::LocalOnly, "Local only");
                    ui.selectable_value(&mut editor.server.network.mode, ConnectionMode::RemoteOnly, "Remote only");
                });
                ui.label(RichText::new("Every address must match this server's saved SSH identity. Confirm it once using Test connection before syncing.").small());
                if !editor.server.network.tailscale_host.is_empty() {
                    field(ui, "Tailscale address", &mut editor.server.network.tailscale_host, "", false);
                    ui.horizontal(|ui| { ui.label("Tailscale SSH port"); ui.add(egui::DragValue::new(&mut editor.server.network.tailscale_port).range(1..=65535)); });
                }
                if ui.button("Set up Tailscale").clicked() { save = true; tailscale = true; }
                ui.label(RichText::new("Saves these settings and prepares this host and server. Existing SSH access is required. Sign into the same Tailscale network when prompted.").small());
                field(ui, "SSH username", &mut editor.server.user, "user", false);
                ui.separator();
                field(ui, "SSH / login password (blank for SSH keys)", &mut editor.credentials.login, "", true);
                ui.checkbox(&mut editor.credentials.same_password, "Use the login password for sudo too");
                if !editor.credentials.same_password { field(ui, "Sudo password", &mut editor.credentials.sudo, "", true); }
                ui.label(RichText::new("Passwords stay in memory until the app closes. Sudo is used only for server setup (rsync or Tailscale).").small());
                if let Some(error) = &editor.error { ui.colored_label(Color32::DARK_RED, error); }
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() { save = true; }
                    if ui.button("Save & set up").clicked() { save = true; setup = true; }
                });
            });
            if save {
                editor.server.name = editor.server.name.trim().into();
                editor.server.host = editor.server.host.trim().into();
                editor.server.network.public_host = editor.server.network.public_host.trim().into();
                editor.server.network.tailscale_host =
                    editor.server.network.tailscale_host.trim().into();
                editor.server.user = editor.server.user.trim().into();
                match editor.server.validate() {
                    Ok(()) => {
                        let id = editor.server.id;
                        if let Some(server) = self.data.servers.iter_mut().find(|s| s.id == id) {
                            *server = editor.server;
                        } else {
                            self.data.servers.push(editor.server);
                        }
                        self.credentials.insert(id, editor.credentials);
                        self.selected = Some(Selection::Server(id));
                        self.selected_links.clear();
                        self.persist();
                        if tailscale {
                            self.server_action(id, Action::Tailscale, ctx);
                        } else if setup {
                            self.server_action(id, Action::Setup, ctx);
                        }
                    }
                    Err(e) => {
                        editor.error = Some(e);
                        self.server_editor = Some(editor);
                    }
                }
            } else if open {
                self.server_editor = Some(editor);
            }
        }
        if self.new_folder {
            let mut open = true;
            egui::Window::new("Add local folder")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .default_width(450.0)
                .show(ctx, |ui| {
                    field(
                        ui,
                        "Name (optional)",
                        &mut self.folder_name,
                        "Class notes",
                        false,
                    );
                    field(
                        ui,
                        "Local directory",
                        &mut self.folder_path,
                        "/home/user/notes",
                        false,
                    );
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(self.picker.is_none(), egui::Button::new("Browse..."))
                            .clicked()
                        {
                            self.browse(ctx);
                        }
                        if ui.button("Add folder").clicked() {
                            self.add_folder();
                        }
                    });
                    if let Some(error) = &self.error {
                        ui.colored_label(Color32::DARK_RED, error);
                    }
                });
            self.new_folder &= open;
        }
        if let Some(mut editor) = self.link_editor.take() {
            let mut open = true;
            let mut save = false;
            egui::Window::new("Folder / server link")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .default_width(450.0)
                .show(ctx, |ui| {
                    ui.label("Local folder");
                    egui::ComboBox::from_id_salt("link-folder")
                        .selected_text(
                            self.data
                                .folders
                                .iter()
                                .find(|f| f.id == editor.link.folder)
                                .map_or("Choose folder", |f| &f.name),
                        )
                        .show_ui(ui, |ui| {
                            for folder in &self.data.folders {
                                ui.selectable_value(
                                    &mut editor.link.folder,
                                    folder.id,
                                    &folder.name,
                                );
                            }
                        });
                    ui.label("Server");
                    egui::ComboBox::from_id_salt("link-server")
                        .selected_text(
                            self.data
                                .servers
                                .iter()
                                .find(|s| s.id == editor.link.server)
                                .map_or("Choose server", |s| &s.name),
                        )
                        .show_ui(ui, |ui| {
                            for server in &self.data.servers {
                                ui.selectable_value(
                                    &mut editor.link.server,
                                    server.id,
                                    &server.name,
                                );
                            }
                        });
                    field(
                        ui,
                        "Remote directory",
                        &mut editor.link.remote,
                        "Leave empty for ~/codesync on the server",
                        false,
                    );
                    ui.label("Leave empty to use codesync in the server user's home directory. It is created on the first sync.");
                    ui.label("Adding a link does not move or transfer files.");
                    if let Some(error) = &editor.error {
                        ui.colored_label(Color32::DARK_RED, error);
                    }
                    if ui.button("Save link").clicked() {
                        save = true;
                    }
                });
            if save {
                editor.link.remote = store::remote_directory(&editor.link.remote);
                match self.data.validate_link(&editor.link) {
                    Ok(()) => {
                        self.selected_links.clear();
                        self.selected_links.insert(editor.link.id);
                        if let Some(link) =
                            self.data.links.iter_mut().find(|l| l.id == editor.link.id)
                        {
                            *link = editor.link;
                        } else {
                            self.data.links.push(editor.link);
                        }
                        self.persist();
                    }
                    Err(e) => {
                        editor.error = Some(e);
                        self.link_editor = Some(editor);
                    }
                }
            } else if open {
                self.link_editor = Some(editor);
            }
        }
        if self.host_password.is_some() {
            let mut answer = None;
            egui::Modal::new(egui::Id::new("host-password")).show(ctx, |ui| {
                ui.set_max_width(430.0);
                ui.heading("Host administrator password");
                ui.label(format!("Enter this host's sudo password to set up {}. It is used only for this step and is not saved.", self.host_password_purpose));
                field(ui, "Host password", &mut self.host_password_input, "", true);
                ui.horizontal(|ui| {
                    if ui.button("Continue").clicked() { answer = Some(true); }
                    if ui.button("Cancel").clicked() { answer = Some(false); }
                });
            });
            if let Some(accepted) = answer {
                let password = std::mem::take(&mut self.host_password_input);
                if let Some(sender) = self.host_password.take() {
                    let _ = sender.send(if accepted { Some(password) } else { None });
                }
            }
        }
        if let Some((prompt, _)) = &self.trust {
            let prompt = prompt.clone();
            let mut answer = None;
            egui::Modal::new(egui::Id::new("trust-host")).show(ctx, |ui| {
                ui.set_max_width(550.0);
                ui.heading("Confirm server identity");
                ui.label(&prompt);
                ui.label("Check this fingerprint against your server before trusting it.");
                ui.horizontal(|ui| {
                    if ui.button("Trust this server").clicked() {
                        answer = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        answer = Some(false);
                    }
                });
            });
            if let Some(answer) = answer
                && let Some((_, sender)) = self.trust.take()
            {
                let _ = sender.send(answer);
            }
        }
    }
}
fn ui_home_select_all(
    ui: &mut egui::Ui,
    selected: &mut HashSet<u64>,
    ids: impl Iterator<Item = u64>,
) {
    ui.horizontal(|ui| {
        if ui.small_button("Select all").clicked() {
            selected.extend(ids);
        }
        if ui.small_button("Clear").clicked() {
            selected.clear();
        }
    });
}
fn field(ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str, password: bool) {
    ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .hint_text(hint)
            .password(password)
            .desired_width(f32::INFINITY),
    );
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.poll();
        if self.job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let busy = self.job.is_some();
        let dialog = self.connections
            || self.server_editor.is_some()
            || self.link_editor.is_some()
            || self.new_folder;
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.strong("Codesync");
                if ui
                    .add_enabled(!busy && !dialog, egui::Button::new("Connections"))
                    .clicked()
                {
                    self.connections = true;
                }
                if ui
                    .add_enabled(!dialog, egui::Button::selectable(self.home, "Home"))
                    .clicked()
                {
                    self.home = true;
                }
                ui.separator();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label("File Synchronization");
                });
            });
        });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if busy {
                    ui.spinner();
                }
                ui.label(&self.status);
                ui.separator();
                ui.label(format!(
                    "{} servers | {} folders | {} links",
                    self.data.servers.len(),
                    self.data.folders.len(),
                    self.data.links.len()
                ));
            });
        });
        if !self.home {
            egui::SidePanel::left("navigation")
                .default_width(210.0)
                .min_width(150.0)
                .resizable(true)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.add_enabled_ui(!busy && !dialog, |ui| {
                            ui.strong("Local folders");
                            ui.separator();
                            for folder in &self.data.folders {
                                let selection = Selection::Folder(folder.id);
                                if ui
                                    .selectable_label(
                                        self.selected == Some(selection),
                                        &folder.name,
                                    )
                                    .on_hover_text(folder.path.display().to_string())
                                    .clicked()
                                {
                                    self.selected = Some(selection);
                                    self.selected_links.clear();
                                }
                            }
                            if self.data.folders.is_empty() {
                                ui.label("No folders added");
                            }
                            if ui.button("Add folder...").clicked() {
                                self.new_folder = true;
                                self.error = None;
                            }
                            ui.add_space(18.0);
                            ui.strong("Servers");
                            ui.separator();
                            for server in &self.data.servers {
                                let selection = Selection::Server(server.id);
                                if ui
                                    .selectable_label(
                                        self.selected == Some(selection),
                                        &server.name,
                                    )
                                    .clicked()
                                {
                                    self.selected = Some(selection);
                                    self.selected_links.clear();
                                }
                            }
                            if self.data.servers.is_empty() {
                                ui.label("No servers added");
                            }
                            if ui.button("Add server...").clicked() {
                                self.edit_server(None);
                            }
                        });
                    });
                });
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_enabled_ui(!dialog, |ui| {
                    if self.home {
                        self.show_home(ui, ctx);
                    } else {
                        self.show_details(ui, ctx);
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.strong("Activity");
                    if busy
                        && ui.button("Stop").clicked()
                        && let Some(job) = &self.job
                    {
                        job.cancel();
                        self.status = "Stopping...".into();
                    }
                });
                if let Some(error) = &self.error {
                    ui.colored_label(Color32::DARK_RED, error);
                }
                for (label, url) in &self.logins {
                    if ui
                        .button(format!("Sign in to Tailscale - {label}"))
                        .clicked()
                    {
                        ctx.open_url(egui::OpenUrl::new_tab(url));
                    }
                }
                egui::Frame::new()
                    .fill(Color32::WHITE)
                    .stroke(egui::Stroke::new(1.0_f32, Color32::GRAY))
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        egui::ScrollArea::vertical()
                            .id_salt("activity")
                            .stick_to_bottom(true)
                            .max_height(220.0)
                            .show(ui, |ui| {
                                ui.set_min_height(90.0);
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(if self.output.text.is_empty() {
                                            "No operations yet."
                                        } else {
                                            &self.output.text
                                        })
                                        .monospace()
                                        .size(12.0),
                                    )
                                    .wrap(),
                                );
                            });
                    });
            });
        });
        self.show_dialogs(ctx);
    }
}
