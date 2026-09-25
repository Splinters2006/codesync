# codesync

A Rust CLI for editing code and notes locally, syncing to your server, and running
commands there over SSH. Markdown notes and other regular files sync with code.

## Requirements

Linux or macOS (Windows via WSL), with SSH and rsync on the laptop. The server needs
an SSH server, rsync, a POSIX shell, and the tools for running your code.
Your server must be reachable over SSH from class, for example through an existing
VPN. Turning it on alone does not make it reachable outside your home network.

## Install and use

Build and install from this repository:

```sh
cargo install --path .
```

Then run from the project folder you want to sync:

```sh
codesync init you@your-server /home/you/classwork
codesync push --dry-run
codesync push
codesync run cargo test
codesync shell
# After editing on the server:
codesync pull --dry-run
codesync pull
```

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

This MVP has no background syncing, web editor, or server provisioning.

## Development

```sh
cargo test
cargo clippy -- -D warnings
```
