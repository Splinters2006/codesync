use eframe::egui::{self, RichText};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::mpsc,
};

const MAX_ENTRIES: usize = 10_000;
const PREVIEW_BYTES: u64 = 256 * 1024;
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Directory,
    File,
    Link,
    Other,
}
struct Entry {
    name: std::ffi::OsString,
    kind: Kind,
    size: u64,
}
struct Listing {
    relative: PathBuf,
    entries: Vec<Entry>,
    limited: bool,
}
struct Preview {
    name: String,
    text: String,
    truncated: bool,
}
pub struct Browser {
    root: PathBuf,
    relative: PathBuf,
    entries: Vec<Entry>,
    listing: Option<mpsc::Receiver<Result<Listing, String>>>,
    loading_preview: Option<mpsc::Receiver<Result<Preview, String>>>,
    preview: Option<Preview>,
    show_hidden: bool,
    limited: bool,
    error: Option<String>,
}
impl Browser {
    pub fn new(root: PathBuf, ctx: &egui::Context) -> Self {
        let mut browser = Self {
            root,
            relative: PathBuf::new(),
            entries: Vec::new(),
            listing: None,
            loading_preview: None,
            preview: None,
            show_hidden: false,
            limited: false,
            error: None,
        };
        browser.load(PathBuf::new(), ctx);
        browser
    }
    fn load(&mut self, relative: PathBuf, ctx: &egui::Context) {
        let (send, receive) = mpsc::channel();
        let root = self.root.clone();
        let ctx = ctx.clone();
        self.listing = Some(receive);
        self.preview = None;
        self.loading_preview = None;
        self.error = None;
        std::thread::spawn(move || {
            let _ = send.send(read_directory(&root, &relative));
            ctx.request_repaint();
        });
    }
    fn preview_file(&mut self, relative: PathBuf, ctx: &egui::Context) {
        let (send, receive) = mpsc::channel();
        let root = self.root.clone();
        let ctx = ctx.clone();
        self.loading_preview = Some(receive);
        self.preview = None;
        self.error = None;
        std::thread::spawn(move || {
            let _ = send.send(read_preview(&root, &relative));
            ctx.request_repaint();
        });
    }
    pub fn show(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(receiver) = &self.listing
            && let Ok(result) = receiver.try_recv()
        {
            self.listing = None;
            match result {
                Ok(listing) => {
                    self.relative = listing.relative;
                    self.entries = listing.entries;
                    self.limited = listing.limited;
                }
                Err(error) => self.error = Some(error),
            }
        }
        if let Some(receiver) = &self.loading_preview
            && let Ok(result) = receiver.try_recv()
        {
            self.loading_preview = None;
            match result {
                Ok(preview) => self.preview = Some(preview),
                Err(error) => self.error = Some(error),
            }
        }
        ui.separator();
        ui.strong("Folder contents")
            .on_hover_text(format!("Viewing saved directory: {}", self.root.display()));
        ui.label(if self.relative.as_os_str().is_empty() {
            "/".into()
        } else {
            format!("/{}", self.relative.display())
        });
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    self.listing.is_none() && !self.relative.as_os_str().is_empty(),
                    egui::Button::new("Up"),
                )
                .clicked()
            {
                self.load(
                    self.relative.parent().unwrap_or(Path::new("")).to_owned(),
                    ctx,
                );
            }
            if ui
                .add_enabled(self.listing.is_none(), egui::Button::new("Refresh files"))
                .clicked()
            {
                self.load(self.relative.clone(), ctx);
            }
            ui.checkbox(&mut self.show_hidden, "Show hidden files");
        });
        if self.listing.is_some() {
            ui.label("Loading files...");
        }
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::DARK_RED, error);
        }
        let visible: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| self.show_hidden || !entry.name.to_string_lossy().starts_with('.'))
            .collect();
        ui.label(format!("{} entries", visible.len()));
        if self.limited {
            ui.label("Showing at most 10,000 entries in this directory.");
        }
        let mut selected = None;
        egui::Frame::new()
            .fill(egui::Color32::WHITE)
            .inner_margin(6.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("folder-files")
                    .max_height(230.0)
                    .show_rows(ui, 26.0, visible.len(), |ui, rows| {
                        for index in rows {
                            let entry = visible[index];
                            ui.push_id(index, |ui| {
                                ui.horizontal(|ui| {
                                    let kind = match entry.kind {
                                        Kind::Directory => "Folder",
                                        Kind::File => "File",
                                        Kind::Link => "Link",
                                        Kind::Other => "Other",
                                    };
                                    ui.label(kind);
                                    let width = (ui.available_width() - 90.0).max(40.0);
                                    if ui
                                        .add_sized(
                                            [width, 22.0],
                                            egui::Button::new(entry.name.to_string_lossy())
                                                .truncate(),
                                        )
                                        .on_hover_text(entry.name.to_string_lossy())
                                        .clicked()
                                    {
                                        selected =
                                            Some((self.relative.join(&entry.name), entry.kind));
                                    }
                                    if entry.kind == Kind::File {
                                        ui.label(size_label(entry.size));
                                    }
                                });
                            });
                        }
                    });
                if visible.is_empty() && self.listing.is_none() {
                    ui.label("No files to show.");
                }
            });
        if let Some((path, kind)) = selected
            && self.listing.is_none()
        {
            match kind {
                Kind::Directory => self.load(path, ctx),
                Kind::File => self.preview_file(path, ctx),
                _ => {
                    self.error = Some(
                        "Links and special files are listed but not opened in the viewer.".into(),
                    )
                }
            }
        }
        if self.loading_preview.is_some() {
            ui.label("Loading preview...");
        }
        if let Some(preview) = &self.preview {
            ui.separator();
            ui.strong(&preview.name);
            if preview.truncated {
                ui.label("Preview limited to the first 256 KiB.");
            }
            egui::ScrollArea::both()
                .id_salt("file-preview")
                .max_height(230.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(&preview.text).monospace()).selectable(true),
                    );
                });
        }
        ui.label(
            RichText::new("Read-only viewer. Select a folder to browse or a text file to preview.")
                .small(),
        );
    }
}
fn size_label(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    }
}
fn checked_path(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    if relative
        .components()
        .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err("Choose a location inside this folder.".into());
    }
    let root = root
        .canonicalize()
        .map_err(|e| format!("Cannot open folder: {e}"))?;
    let mut path = root.clone();
    for component in relative.components() {
        path.push(component);
        if fs::symlink_metadata(&path)
            .map_err(|e| format!("Cannot read entry: {e}"))?
            .file_type()
            .is_symlink()
        {
            return Err("The viewer does not follow symbolic links.".into());
        }
    }
    let path = path
        .canonicalize()
        .map_err(|e| format!("Cannot open entry: {e}"))?;
    if !path.starts_with(root) {
        return Err("This entry is outside the selected folder.".into());
    }
    Ok(path)
}
fn read_directory(root: &Path, relative: &Path) -> Result<Listing, String> {
    let path = checked_path(root, relative)?;
    let mut entries = Vec::new();
    let mut limited = false;
    for entry in fs::read_dir(path).map_err(|e| format!("Cannot list directory: {e}"))? {
        if entries.len() == MAX_ENTRIES {
            limited = true;
            break;
        }
        let entry = entry.map_err(|e| e.to_string())?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
        let kind = if metadata.is_dir() {
            Kind::Directory
        } else if metadata.is_file() {
            Kind::File
        } else if metadata.file_type().is_symlink() {
            Kind::Link
        } else {
            Kind::Other
        };
        entries.push(Entry {
            name: entry.file_name(),
            kind,
            size: metadata.len(),
        });
    }
    entries.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.name.cmp(&b.name)));
    Ok(Listing {
        relative: relative.into(),
        entries,
        limited,
    })
}
fn read_preview(root: &Path, relative: &Path) -> Result<Preview, String> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = checked_path(root, relative)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| format!("Cannot open file: {e}"))?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Only regular files can be previewed.".into());
    }
    let mut bytes = Vec::new();
    file.take(PREVIEW_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Cannot read file: {e}"))?;
    let truncated = bytes.len() as u64 > PREVIEW_BYTES;
    bytes.truncate(PREVIEW_BYTES as usize);
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => Some(text),
        Err(error) if truncated && error.error_len().is_none() => {
            std::str::from_utf8(&bytes[..error.valid_up_to()]).ok()
        }
        _ => None,
    };
    let text = text
        .filter(|text| {
            !text
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        })
        .map(str::to_owned)
        .unwrap_or_else(|| "Binary or non-UTF-8 file — no text preview available.".into());
    Ok(Preview {
        name: relative.display().to_string(),
        text,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lists_folders_first_and_previews_without_modifying_files() {
        let scratch = crate::sync::Scratch::new().unwrap();
        fs::create_dir(scratch.0.join("notes")).unwrap();
        fs::write(scratch.0.join("notes/readme.txt"), "Hello, 世界!\n").unwrap();
        fs::write(scratch.0.join(".hidden"), "hidden").unwrap();
        fs::write(scratch.0.join("data.bin"), [0u8, 255, 1]).unwrap();
        let listing = read_directory(&scratch.0, Path::new("")).unwrap();
        assert_eq!(listing.entries.len(), 3);
        assert!(listing.entries[0].kind == Kind::Directory);
        assert_eq!(listing.entries[0].name, "notes");
        assert!(!listing.limited);
        let preview = read_preview(&scratch.0, Path::new("notes/readme.txt")).unwrap();
        assert_eq!(preview.text, "Hello, 世界!\n");
        assert!(!preview.truncated);
        assert!(
            read_preview(&scratch.0, Path::new("data.bin"))
                .unwrap()
                .text
                .contains("Binary")
        );
        assert_eq!(
            fs::read_to_string(scratch.0.join("notes/readme.txt")).unwrap(),
            "Hello, 世界!\n"
        );
        assert!(read_preview(&scratch.0, Path::new("missing")).is_err());
    }
    #[test]
    fn bounds_previews_and_refuses_links_and_parent_navigation() {
        let scratch = crate::sync::Scratch::new().unwrap();
        let mut text = vec![b'a'; PREVIEW_BYTES as usize - 1];
        text.extend_from_slice("é".as_bytes());
        fs::write(scratch.0.join("large.txt"), text).unwrap();
        let preview = read_preview(&scratch.0, Path::new("large.txt")).unwrap();
        assert!(preview.truncated);
        assert_eq!(preview.text.len(), PREVIEW_BYTES as usize - 1);
        std::os::unix::fs::symlink("large.txt", scratch.0.join("link")).unwrap();
        assert!(read_preview(&scratch.0, Path::new("link")).is_err());
        assert!(read_directory(&scratch.0, Path::new("../")).is_err());
        assert!(read_directory(&scratch.0, Path::new("/tmp")).is_err());
    }
}
