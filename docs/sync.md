# Two-way sync on Linux

Use **Connections** to add a reachable Linux host using its SSH address and
account. On a computer that should receive connections, **Prepare this host to
receive connections** installs SSH, rsync, and SHA-256 tools and starts SSH.
This needs local administrator authorization. Both computers must be online;
a central server is optional. Connections use existing SSH accounts and pinned
host fingerprints, not pairing codes. SSH access has the account's normal
permissions; the folder selection is not an access-control sandbox.

On Home, check folders on the left and hosts/servers on the right, then **Sync**.
New links use `~/codesync/<folder name>`. Existing links retain their paths.

## Content rules

Files are compared by their relative path and SHA-256 content hash, not their
modification time:

- Equal hashes: retain the file without creating duplicates.
- A file exists on only some participants: copy it to the others.
- Different hashes at the same path: preserve each distinct version using a
  host/server prefix, such as `Workstation_notes.txt` and `Home_notes.txt`.
  The conflicting original names are renamed; versions are not overwritten.
- A prefixed name already exists: add a hash fragment and counter to avoid
  collisions. Duplicate display names are handled the same way.
- A file and directory occupy the same path: stop and ask you to rename one.

Files remain in their original subdirectories. The local prefix uses
`/etc/hostname`; remote prefixes use Codesync display names. Prefix punctuation
is normalized. All selected server copies of a local folder participate in one
merge, so remote-only versions reach the other selected servers too.

## Limits and recovery

Sync is manual. It does not infer deletions, pick the newest version, merge text,
or choose which conflict copy you prefer. Missing files are restored from other
participants. Resolve unwanted copies on every participant if you want them gone.
Pause editing while syncing. Conflict renames check the source hash again and
refuse to replace an existing destination. Final checksum verification reports
changes during transfer; retry when edits have stopped.

This implementation downloads temporary snapshots of every selected remote copy
and builds a merged copy locally before applying changes. It needs enough local
temporary space for those copies and transfers remote snapshots on every run.
Temporary files use a private directory and are cleaned up on normal completion,
cancellation, or failure. An abrupt process kill can leave the private temporary
directory behind. Regular files and directories are supported; symlinks and
special files stop the merge. The usual Codesync exclusions still apply.

A batch is not an atomic transaction across machines. If interrupted, completed
copies and conflict renames remain. Retry Sync to bring the remaining copies up
to date. Keep backups for important work.

The CLI's `push` and `pull` retain their explicit one-way behavior.
