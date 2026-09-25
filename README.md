# codesync

A Rust CLI for editing code and notes locally, syncing to your server, and running
commands there over SSH. Markdown notes and other regular files sync with code.

## Requirements

Linux or macOS (Windows via WSL), with Rust/Cargo, SSH, and rsync on the laptop.
The server needs working SSH access and a POSIX shell. `codesync init` installs
rsync on supported servers if needed; project tools such as Rust must be installed
separately.
Your server must be reachable over SSH from class, for example through an existing
VPN. Turning it on alone does not make it reachable outside your home network.

## Install and use

Build and install from this repository:

```sh
cargo install --path .
```

The `.` means the current repository folder. If Bash cannot find `codesync`, run:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
codesync --help
```

Add the export line to `~/.bashrc` to keep it for new Bash terminals.

Then run from the project folder you want to sync:

```sh
codesync init you@your-server /home/you/classwork
# init saves settings and prepares the server; no separate setup step needed.
codesync push --dry-run
codesync push
codesync run cargo test
codesync shell
# After editing on the server:
codesync pull --dry-run
codesync pull
```

## Server setup

`init` saves your connection settings, then checks for rsync over SSH and installs
it if missing. It supports Alpine
(`apk`), Debian/Ubuntu (`apt-get`), Fedora (`dnf`), and Arch (`pacman`). It uses
root, sudo, or doas, with an interactive terminal for password prompts. Package
installation is automatic during init; Debian/Ubuntu also refreshes package lists.
If setup fails, the config stays saved. Run `codesync setup` to retry (also use
this for projects configured with an older version). Both commands skip installation
if rsync is already available. Failures display the package manager's error.

SSH access must already work. Setup installs the sync dependency only; install
project tools such as Rust separately. Unsupported servers get manual-install
guidance.

## Updating an existing installation

From this repository, rebuild and replace the installed CLI:

```sh
cargo install --path . --force
```

Then, from your already configured project folder, prepare the server:

```sh
codesync setup
codesync push
```

Keep your existing `.codesync` file. There is no need to run `init` again.

## Running code and opening a shell

`run` uploads first, then executes the supplied program and arguments in the remote
folder. Arguments are literal; for shell syntax use
`codesync run sh -lc 'command1 && command2'`. Remote tools must be on the SSH
session's PATH. `shell` opens an interactive remote shell without uploading.
Use a server-side terminal multiplexer for work that should survive disconnects.

Use an SSH config alias for custom ports, keys, or jump hosts. SSH handles
credentials and host verification. The two-line `.codesync` file stores the host
and remote path. Commands must run from its directory. `init` refuses to overwrite
an existing config; edit it directly to change the destination.

## Sync behavior

Sync is manual and one-way per command. Work locally and push, or work remotely
and pull before editing locally again. Rsync compares file size and modification
time. There is no conflict detection or merging: push can overwrite remote edits,
and pull can overwrite local edits. Keep Git history or backups. Deletions are
never propagated. Use a dedicated remote directory for each project.

Excluded at any depth: `.git`, `target`, `node_modules`, `.codesync`, `.env`,
`.env.*`, `*.pem`, `*.key`. Other files are included; review before uploading.
Symlinks are copied as links. Dry runs do not create remote directories, so an
initial preview can fail if the remote parent does not exist yet.

This MVP has no background syncing or web editor.

## Development

```sh
cargo test
cargo clippy -- -D warnings
```
