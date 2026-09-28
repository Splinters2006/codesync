Codesync for Windows (x64)

1. Extract the entire ZIP to a folder of your choice.
2. Install 64-bit Cygwin from https://cygwin.com/install.html, selecting
   openssh, rsync, and coreutils. Use C:\cygwin64 and its default /cygdrive prefix.
3. Double-click codesync-gui.exe. No Rust, Cargo, or source checkout is needed.
   The GUI needs an OpenGL-capable graphics driver.

For a custom Cygwin folder, set the user environment variable CODESYNC_CYGWIN_BIN
to its bin directory, then reopen Codesync. Cygwin is not bundled in this ZIP.
The optional codesync.exe is the command-line app; run it from a terminal.

Servers remain Linux-only. Windows can connect and sync as a host but cannot
offer incoming SSH setup. See https://github.com/Splinters2006/codesync/blob/main/docs/windows.md
for setup details and Windows filename restrictions.

Profiles and the encrypted password vault are kept in %APPDATA%\codesync.
SSH identities are kept in %USERPROFILE%\.ssh\codesync_known_hosts.
These files are separate from the downloaded app. The vault asks for a master
password, which is never saved and cannot be recovered if forgotten.

To update, close Codesync and replace the executables with a newer download.
The "codesync update" command is for source installations with Rust and Git.
To remove the app, delete this extracted folder. Your saved settings remain.
