# Windows hosts

The GUI and CLI run natively on Windows and connect to Linux servers. WSL is not
required. Windows hosts do not offer incoming SSH setup or act as Codesync servers.

## Download the Windows executable

1. Download the **codesync-windows-x64** artifact from a successful
   [Windows executable workflow run](https://github.com/Splinters2006/codesync/actions/workflows/windows-package.yml).
   GitHub artifact downloads require signing in. Extract the artifact, then
   extract the enclosed `codesync-windows-x64.zip`.
2. Install [64-bit Cygwin](https://cygwin.com/install.html), selecting **openssh**,
   **rsync**, and **coreutils**. Use `C:\cygwin64` and its default `/cygdrive`
   mount prefix. These transfer tools are required and are not bundled.
3. Double-click **codesync-gui.exe** in the extracted folder. You can create a
   desktop shortcut to it. The app requires an OpenGL-capable graphics driver.

No Rust, Cargo, Visual Studio, or source checkout is needed to use this download.
The package targets x64 Windows and also includes `codesync.exe` for optional
terminal use. Its MSVC build includes the C runtime statically. A SHA-256 checksum
is included alongside the ZIP. Builds are currently unsigned.

For a custom Cygwin folder, set the user environment variable
`CODESYNC_CYGWIN_BIN` to its `bin` directory in Windows Environment Variables,
then reopen Codesync. Adding Cygwin to PATH is unnecessary.

To update, close the app and replace the executables with a newer download.
Profiles and encrypted passwords stay in `%APPDATA%\codesync`. To remove the app,
delete the extracted folder; saved settings remain in your user profile.
Linux continues to use `./install.sh` from a terminal.

## Install from source

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

Passwords can be saved in `%APPDATA%\codesync\.codesync-passwords.enc`, encrypted
with a master password that you enter once per session. See [encrypted local passwords](authentication.md#encrypted-local-passwords). The GUI authenticates its askpass helper over a
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

For source installations, close the GUI before `codesync update`. The Windows updater pulls the clean Git
checkout and launches the PowerShell installer, which waits for the CLI to exit
before replacing it. Keep the terminal open and check the installer's output;
the initial command's exit status reports that the installer was launched, not
that installation finished.

## Build the Windows package

On a Windows development machine with Python 3.10+, Rust's MSVC toolchain, and
Visual Studio C++ build tools, run:

```powershell
python scripts/package_windows.py
python tests/windows_package.py
```

Output is `dist/codesync-windows-x64/` plus `dist/codesync-windows-x64.zip` and its
`.sha256` checksum. Only executables and the included README enter the archive;
local settings and passwords are never packaged. `dist/` is ignored by Git.
The package test checks the archive, executable architecture/subsystems, CLI,
and the GUI password helper's output pipe.

The GitHub **Windows executable** workflow builds and tests this package with the
MSVC toolchain. It runs on pushes to `main`, `v*` tags, pull requests, and manually
through **Run workflow**. Downloads are workflow artifacts; building does not
publish a GitHub Release.

Cross-compiling on Linux is also supported: install the Rust target
`x86_64-pc-windows-gnu` and a MinGW-w64 toolchain, put its tools on PATH, then run
`python3 scripts/package_windows.py --target x86_64-pc-windows-gnu`.
Set `CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER` if the compiler is not named
`x86_64-w64-mingw32-gcc`. With LLVM-MinGW, use the Rust target
`x86_64-pc-windows-gnullvm` instead, pass that to `--target`, and set
`CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER` to its `x86_64-w64-mingw32-clang`.
Runtime smoke tests must still run on Windows.
