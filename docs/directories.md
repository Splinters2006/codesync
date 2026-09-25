# Syncing multiple directories

The GUI opens on **Home**, with folders on the left and servers on the right.
Check any number of each and click **Sync** to synchronize every selected
folder with every selected server in both directions. New links use
`~/codesync/<local directory name>`; existing links keep their destinations. Conflicting
folder names require distinct custom destinations before the batch can start.

Use **Open** beside an item to manage its links in a settings popup, or
**Add link** in that popup for a custom destination. One folder can link to multiple servers, and one server can link to multiple
folders. Check the links to include, then use **Sync**. Files with different hashes are kept with host/server prefixes. Use **Select all** to include every visible link.
Leave **Remote directory** empty, or enter `codesync`, to use
`~/codesync/<local directory name>` in the server user's home directory. For
example, `test4` syncs to `~/codesync/test4`. `/srv/codesync` similarly becomes
`/srv/codesync/test4`. Codesync creates it on the first sync. For additional
local folders on the same server, enter distinct absolute destinations to avoid
mixing their contents.

GUI links are stored in `profiles.json` and do not replace `.codesync` files.

## Command line

Each local directory has its own `.codesync` file. Install the CLI once, then
configure each folder separately with a distinct remote destination:

```sh
mkdir -p ~/classwork ~/notes
cd ~/classwork
codesync init user@192.168.0.6 /home/user/classwork
codesync push

cd ~/notes
codesync init user@192.168.0.6 /home/user/notes
codesync push
```

Run `push`, `pull`, `run`, or `shell` from the folder you want to work with.
Each sync includes that folder's subdirectories. The commands sync one configured
folder at a time; the CLI has no global registry or sync-all command.

## Fixing a destination

From the local folder whose destination is wrong:

```sh
codesync init --force user@192.168.0.6 /home/user/TEST
codesync push --dry-run
codesync push
```

`--force` replaces this folder's saved connection settings and runs server setup.
It does not move or delete previously uploaded files. Setup failures leave the new
settings saved; retry with `codesync setup`. Without `--force`, init preserves an
existing config.

If you configured the wrong *local* folder, enter the intended local folder and
run init there. The old folder's config remains independent. Avoid mapping two
local folders to the same remote destination, since uploads can overwrite files.

To install or update with automatic PATH setup, run `./install.sh` from the
codesync source repository. Open a new terminal afterward.
