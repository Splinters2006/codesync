# Windows hosts

The GUI and CLI run natively on Windows and connect to Linux servers. WSL is not
required. Windows hosts do not offer incoming SSH setup or act as Codesync servers.

## Install

1. Install native Windows [Rust/Cargo](https://rustup.rs/) with the MSVC toolchain
   and Visual Studio C++ build tools. The GUI requires an OpenGL-capable driver.
2. Install [64-bit Cygwin](https://cygwin.com/install.html), selecting **openssh**,
   **rsync**, and **coreutils**. Keep its default `/cygdrive` mount prefix. Codesync
   uses this matched toolchain, including Cygwin SSH, for transfers.
3. From the checkout in PowerShell, run:

   ```powershell
   .\install.ps1
   codesync gui
   ```

If PowerShell's local script policy blocks the installer, invoke it for this
process with `powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1`.
For a custom Cygwin installation, use `-CygwinBin 'D:\Cygwin\bin'`.
The installer remembers that location in the user environment variable
`CODESYNC_CYGWIN_BIN`. For direct Cargo builds, set that variable yourself or use
`C:\cygwin64\bin`. Adding Cygwin to the global PATH is unnecessary.

`-CliOnly` installs just the CLI; `-PathOnly` repairs the installed command's PATH.
`CARGO_INSTALL_ROOT` and `CARGO_HOME` are respected. Open a new terminal for
persistent environment changes to take effect. The installer also updates the
current PowerShell session. No administrator rights are needed by Codesync's
installer itself.

## Use

Enter normal Windows directories, such as `C:\Users\Sam\Class notes`, or choose
one with **Browse**. Drive-letter, extended-length, and UNC paths are translated
for Cygwin; remote directories remain Linux paths such as `/home/sam/notes`.
Keep shared folders on NTFS: preserving conflicts uses hard links before removing
an original name, and fails safely if the filesystem does not support them.

Passwords remain in memory. The GUI authenticates its askpass helper over a
loopback socket using a fresh random token and still requires explicit SSH
fingerprint confirmation. GUI identities are saved in
`%USERPROFILE%\.ssh\codesync_known_hosts`; profiles are saved in
`%APPDATA%\codesync\profiles.json`. SSH keys, agents, and SSH configuration use
Cygwin OpenSSH's conventions, rather than the separate Windows OpenSSH agent.
Connection multiplexing is disabled on Windows.

Windows transfers do not preserve Linux ownership or permission bits. Two-way
sync and Windows CLI transfers inspect source filenames before copying. Unsupported
characters, reserved device names, trailing spaces/dots, and case-only collisions
stop the operation. Rename those entries on Linux and retry. Across participants,
GUI sync also rejects case-only collisions before conflict renames. Links and
special files are unsupported. Files must remain unchanged while syncing.
The GUI limits the preflight listing to about 2 MB and refuses incomplete listings.

Network scanning uses connected IPv4 interfaces. For Tailscale, install and sign
in to the Windows Tailscale app first, make `tailscale.exe` available on PATH,
then use Codesync's normal server connection/setup controls. Server-side setup
continues to use the saved Linux sudo credentials.

Close the GUI before `codesync update`. The Windows updater pulls the clean Git
checkout and launches the PowerShell installer, which waits for the CLI to exit
before replacing it. Keep the terminal open and check the installer's output;
the initial command's exit status reports that the installer was launched, not
that installation finished.
