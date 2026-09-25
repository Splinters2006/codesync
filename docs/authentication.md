# SSH logins

The GUI has a password field for each server. It remembers login and sudo
passwords only while the app is open; reopening requires re-entering them in
**Edit server / passwords** unless SSH keys are in use. Setup uses sudo separately
from normal transfers. New hosts require a fingerprint confirmation dialog.

Codesync reuses an authenticated SSH connection for commands to the same server
and user, including rsync transfers and different project folders. You normally
enter your server password once, then reuse that connection until it has been idle
for ten minutes. Disconnecting from the network or restarting the server may
require another login. Passwords are handled by SSH and are not stored by codesync.

The CLI uses OpenSSH's `ControlMaster=auto`, `ControlPersist=10m`, and
`ControlPath=~/.ssh/codesync-%C`. The GUI uses a separate connection socket per
server ID and verifies a shared identity across local, public, and Tailscale
addresses, stored in `~/.ssh/codesync_known_hosts`. Use **Test connection** to
confirm that identity before the first GUI sync. Your local `~/.ssh` directory must exist and be
private to your user. If it does not exist, create it with `mkdir -m 700 ~/.ssh`.
See the [OpenSSH documentation](https://man.openbsd.org/ssh_config#ControlMaster).

To use this change, reinstall from the codesync source directory:

```sh
cargo install --path . --force
```

Then run `codesync push` from your configured project directory.

Server setup may also prompt for sudo or doas authentication. That is separate
from the SSH login used to connect to the server.

You can end a reused CLI connection explicitly:

```sh
ssh -o ControlPath=~/.ssh/codesync-%C -O exit user@192.168.0.6
```

SSH keys and an SSH agent can also avoid server password prompts after the shared
connection expires. Codesync uses your normal SSH configuration and agent.
