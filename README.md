# Codesync

Sync code and notes between local folders and your own servers. Includes a Rust
CLI and a native Linux desktop app with a classic white-and-gray interface.

## Install

The host needs Rust/Cargo, OpenSSH, rsync, and `sha256sum`. The desktop app needs a Wayland or
X11 desktop with OpenGL. Zenity is optional for the folder picker; you can also
enter a path manually. Native Windows support is not implemented.

From this repository:

```sh
./install.sh
```

On every machine, the installer runs Cargo and configures PATH for that user
in Bash, Zsh, or Fish. If an existing `~/.local/bin` or `~/bin` is already on PATH,
it adds launchers there so the command works immediately. Otherwise, open a new
terminal after installation and run `codesync gui` from any directory. The
installer also prints a direct launch command for the current terminal.
Reinstalling does not duplicate PATH entries. No root privileges are needed.
`CARGO_INSTALL_ROOT` and `CARGO_HOME` are respected.

For a CLI-only installation:

```sh
./install.sh --cli-only
```

You can still run `cargo install --path . --force` directly, but Cargo itself does
not update shell startup files. Use the installer for automatic PATH setup.

For an existing installation made with Cargo, repair PATH without rebuilding:

```sh
./install.sh --path-only
```


## Update from Git

After installing a version with the updater, run this from any directory:

```sh
codesync update
```

Codesync remembers the source checkout used to build it, runs
`git pull --ff-only --no-rebase`, and reruns the installer. It preserves the
installation directory and whether you installed the GUI or CLI-only build.
Close and reopen the GUI afterward.

If you moved the checkout or installed from a Git source cache, point to your clone:

```sh
codesync update --repo /path/to/codesync
```

The checkout must be clean and on a branch with a configured upstream. Updates
stop for local changes, untracked files, detached commits, or divergent branches;
they do not reset, stash, or discard your work. Git, Cargo, and the normal build
dependencies are required. If installation fails after pulling, fix the reported
error and rerun the command. Older versions without `update` need `./install.sh`
once to gain the command.

## Add servers and folders

1. Click **Add server**. Enter a name, IP address or hostname, SSH username, and
   port. Enter your SSH login password, or leave it blank to use SSH keys / agent.
2. Keep **Use the login password for sudo too** checked if both passwords are the
   same. Otherwise enter the separate sudo password. Choose **Save & set up** to
   install rsync if needed, or **Save** if the server is already prepared.
3. Click **Add folder** and choose an existing local directory.
4. On **Home**, check folders on the left and servers on the right, then click
   **Sync**. Each selected folder is synchronized in both directions across all
   selected servers. New links use `~/codesync/<local directory name>`; existing destinations
   are preserved. First-time connections ask you to verify the server fingerprint.

Use **Open** beside a folder or server to open its settings popup.
Folder settings include its name, local directory, and links. Server settings
include addresses, credentials, connection tests, setup, and links. For a custom
destination, choose **Add link** in folder or server settings and enter an absolute
remote directory. Press **Escape** to close the focused popup.

The **File browser** is in the center of Home. Drag a folder or server name into
it, or click its **Files** button. Local folders open at their saved directory;
servers open at the SSH account's home directory. Remote browsing uses the saved
credentials, Tailscale preference, and SSH fingerprint checks.
Select a subfolder to browse it or a text file for a read-only preview. The viewer
has **Up**, **Refresh files**, file sizes, and **Show hidden files** controls. It
lists folders first and limits text previews to 256 KiB. Binary files and symbolic
links are listed but not opened. Dropping a server opens its files; it does not
start a sync.

Home includes **Remove** buttons for folders and servers. They remove the entry
and its links from Codesync, leaving all files in place. The **Hosts** section on the right
shows this computer's **Accept SSH connections** checkbox. Checking it installs
and enables SSH; unchecking it stops and disables the system SSH service and any
SSH socket activation. Existing SSH sessions may remain open. The switch reflects
actual service status and refreshes periodically. Enabling SSH requires
administrator authorization. Host preparation installs a root-owned stop helper
and a per-user sudo rule limited to that helper, so switching SSH off does not
prompt for a password. After updating an older installation, run **Connections >
Prepare this host** once to install that permission. Each computer controls its own switch.

Close the popup to continue on Home. Use **Connections** to connect another Linux
host or prepare this host to receive SSH connections. A central server is optional;
both hosts must be online and reachable. Preparation installs OpenSSH server, rsync,
and SHA-256 tools and enables SSH at startup. It preserves SSH configuration and
firewall rules. SSH grants the connected account its normal filesystem permissions.

A server can have many folders, and a folder can have many servers. Each link has
its own remote directory. For example:

| Local folder | Server | Remote directory |
| --- | --- | --- |
| `~/classwork` | Home | `/home/user/classwork` |
| `~/classwork` | Lab | `/home/user/classwork` |
| `~/classwork` | Backup | `/home/user/backups/classwork` |
| `~/notes` | Home | `/home/user/notes` |
| `~/projects` | Home | `/home/user/projects` |

Check folders and servers on Home, then click **Sync**. Use **Select all** or
**Clear** above either list to adjust the batch. Manage individual destinations
with **Open**, then **Edit** or **Unlink** beside a link in the settings popup.

Sync compares SHA-256 hashes for files at the same relative path. Equal content
stays unchanged. Missing files are copied to all selected participants. Different
versions become prefixed files, such as `Workstation_file1.txt` and
`HomeServer_file1.txt`, on every participant. The local prefix comes from the
host's hostname; remote prefixes use their Codesync display names. Identical
versions shared by several hosts produce one copy. If a prefixed name already
exists, a hash fragment and counter distinguish it without replacing that file.

All selected copies of one local folder are compared together. Batches stop on
failure; completed copies and conflict renames remain in place. Deletions are
never propagated, and missing files can be restored from another participant.
See [two-way sync](docs/sync.md) for behavior and limitations.

Use the CLI's `codesync run` to run commands or `codesync shell` for interactive
programs. **Stop** cancels local work but
cannot undo transferred files or guarantee that a remote process stops.

Leave a link's **Remote directory** empty, or enter `codesync`, to use
`~/codesync/<local directory name>` on the server. For example, local `test4`
syncs to `~/codesync/test4`. An absolute parent such as `/srv/codesync` becomes
`/srv/codesync/test4`. The actual directory name is used, not its display label.
The directory is created on the first sync. Use distinct destinations for different
local folders on the same server.

Edit a server to update its address or credentials. Edit a link to change its
remote directory. Removing a server, folder, or link does not delete any files;
removing a server or folder also removes its links from the app.

## Find SSH hosts on your network

In the **Hosts** section, click **Scan network...**, choose a connected IPv4 subnet,
and click **Scan**. The default SSH port is 22; enter another port if needed.
Reachable services with an SSH greeting appear in the list. Use **Connect** to
prefill a new connection, or **Settings** for an existing one. Enter credentials
and verify the fingerprint before syncing. Addresses stay inside settings.

Scanning runs in the background and can be stopped. It does not log in or send
credentials. Results last until the next scan or app restart. Scan each network
separately; up to 4096 addresses are allowed per scan. This is not a complete
inventory: blocked, slow, offline, IPv6-only, or differently configured SSH hosts
may not appear. You can still add those hosts manually.

## Local, public, and Tailscale connections

Server settings include separate local / primary and public addresses and ports.
Choose **Automatic**, **Local only**, or **Remote only**. Automatic tries Tailscale,
local, then public, checking each against the server's saved SSH identity. When
Tailscale is already running, Codesync detects uniquely matching peer names and
addresses. For a device configured by LAN IP, it learns the Tailscale address
through the verified SSH connection, verifies SSH over Tailscale, then saves and
prefers it automatically. No manual Tailscale setup is needed if both devices
are already connected. Ambiguous names are not guessed, and Local only remains
local. If Tailscale SSH cannot connect, a verified local/public route can be used. An
unrelated machine at the same private IP is rejected before files are transferred.
Use **Test connection** to confirm the server fingerprint once before the first sync.

**Set up Tailscale** installs and connects Tailscale on your host and the selected
server. It asks for the host sudo password in a dedicated dialog when needed, and uses
saved server sudo credentials remotely. Devices already connected skip installation. Sign-in buttons appear in the app when browser login is needed. Setup
saves the Tailscale address only after verifying SSH over it. Initial SSH access
must already work. See [remote access setup](docs/remote-access.md) for requirements.

## Passwords and setup

Passwords are remembered **only while the app is open**, and never written to the
profile file. After restarting, use **Open** on the server to enter them again,
or use an SSH key and agent. SSH and sudo are separate credentials: the login
password connects to the server; the sudo password is used only for server setup.
Regular file transfers run as the SSH user and need writable remote directories.

There is no generic terminal reply field. OpenSSH gets the login password through
a private local authentication socket. The fixed setup command receives the sudo
password through its SSH input stream. Passwords are not passed in command-line
arguments or environment variables. A new host key requires a dedicated fingerprint
confirmation; changed keys are not automatically trusted. SSH keys requiring a
passphrase should be unlocked in your SSH agent. OTP / interactive MFA is not
supported by this GUI version.

SSH access must already work. Rsync setup supports Alpine, Debian/Ubuntu, Fedora, and
Arch, using root or sudo. Passwordless doas is also supported by the GUI; passworded
doas needs manual setup through a terminal. Install project tools such as Rust
separately. The GUI does not need util-linux `script` anymore.

## Saved settings and existing projects

Servers, folders, and links are stored in
`$XDG_CONFIG_HOME/codesync/profiles.json` (normally
`~/.config/codesync/profiles.json`). Credentials are kept separately in memory.

The previous `folders.json` list is imported and saved when there is no new profile file.
Adding a folder with a `.codesync` file imports its existing server destination.
The GUI does not rewrite `.codesync` when switching servers. The CLI retains its
original single-destination-per-folder behavior, independently of GUI links.

## Command line

From a local project folder:

```sh
codesync init user@192.168.0.6 /home/user/classwork
codesync push --dry-run
codesync push
codesync run cargo test
codesync shell
codesync pull --dry-run
codesync pull
```

`init` saves settings and sets up rsync. Retry setup with `codesync setup`.
Use `codesync init --force USER@HOST /remote/folder` to replace an existing config.
See [directories](docs/directories.md) and [SSH logins](docs/authentication.md).
SSH config aliases can supply keys and jump hosts. The GUI's port field sets the
port explicitly; the CLI uses the port from your SSH config.

## Sync behavior

The GUI uses manual, two-way hash-based sync. The CLI's `push` and `pull` remain
explicit one-way rsync operations that can overwrite edits and compare size and
modification time by default. Keep Git history or backups.

Excluded at any depth: `.git`, `target`, `node_modules`, `.codesync`, `.env`,
`.env.*`, `*.pem`, `*.key`. GUI two-way sync accepts regular files and directories;
symbolic links and special files stop the operation before conflict renaming.
CLI transfers still support symlinks. Dry runs are CLI-only.

For access from class or another network, use a reachable public IPv4/IPv6 address,
a hostname, or a private VPN address. See [remote access setup](docs/remote-access.md)
for Tailscale and public-address options. Turning the server on alone does not
provide access from outside your home network.

## Development

```sh
cargo run --bin codesync-gui
cargo test
cargo clippy --all-targets -- -D warnings
```

Authentication tests use local Unix sockets; restricted sandboxes must allow those.
