# SSH logins

The GUI has a password field for each server. It remembers login and sudo
passwords in an encrypted local vault after you set a master password. Unlock
once after restarting to restore them; SSH keys/agents can be used without
unlocking. Setup uses sudo separately
from normal transfers. New hosts require a fingerprint confirmation dialog.

Codesync reuses an authenticated SSH connection for commands to the same server
and user, including rsync transfers and different project folders. You normally
enter your server password once, then reuse that connection until it has been idle
for ten minutes. Disconnecting from the network or restarting the server may
require another login. The GUI stores passwords only in its encrypted vault; the CLI continues to use SSH authentication directly.

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

On Windows, Codesync uses Cygwin OpenSSH and disables connection multiplexing.
The GUI's dedicated known-hosts file is under `%USERPROFILE%\.ssh`; SSH keys and
agents follow Cygwin's configuration. See [Windows hosts](windows.md).


## Encrypted local passwords

The GUI creates `.codesync-passwords.enc` in the same configuration directory as
`profiles.json`: normally `~/.config/codesync/` on Linux (respecting
`XDG_CONFIG_HOME`) or `%APPDATA%\codesync\` on Windows. Saving the first server
password prompts you to create a master password with at least 12 characters.
The **Saved passwords…** button can also create or unlock the vault. Unlocking
runs in the background so password derivation does not freeze the UI.

The versioned format derives a 256-bit key using
[Argon2id](https://docs.rs/argon2/0.5.3/argon2/) with a random 16-byte salt,
64 MiB of memory, three passes, and one lane. Each save uses
[XChaCha20-Poly1305](https://docs.rs/chacha20poly1305/0.10.1/chacha20poly1305/)
with a fresh random 24-byte nonce; the version, salt, and nonce are authenticated
along with the encrypted JSON payload. Login passwords, sudo passwords, and the
same-password setting are stored by server ID. No master password or unencrypted
key is stored on disk. Host administrator passwords for local setup remain
transient and are not saved.

The decrypted credentials and key remain in memory while unlocked. Saving server
settings rewrites the encrypted file atomically. Removing a server removes its
stored credentials when the unlocked vault is saved; removals made while locked
are applied on the next unlock. **Lock passwords** clears session credentials and
the unlocked key; previously authenticated SSH connections can remain active.
Files use owner-only permissions on Linux and inherit the user configuration
directory's ACL on Windows. Temporary writes contain ciphertext only.

The `.gitignore` pattern and transfer exclusions both cover
`.codesync-passwords.enc*`, including temporary files and the save lock. Keep a
backup of the encrypted file together with its profiles if you need recovery.
There is no master-password recovery: if it is forgotten, move the vault aside
while Codesync is closed, restart, create a new vault, and re-enter the server
passwords. Incorrect master passwords or corrupt files never overwrite the vault.

Only one app instance can write the vault at a time. An instance that detects
another writer's changes refuses to overwrite them; restart and unlock again.
After a crash, if saving reports a stale `.codesync-passwords.enc.lock`, close all
Codesync instances before removing that lock file. Encryption protects the stored
file; it does not protect passwords from software running inside an unlocked app
or a compromised user session. The CLI does not read the GUI vault.
