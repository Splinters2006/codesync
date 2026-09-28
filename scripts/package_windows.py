#!/usr/bin/env python3
"""Build the Windows executables and a portable download (Python 3.10+)."""
import argparse
import hashlib
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--target",
        choices=("x86_64-pc-windows-msvc", "x86_64-pc-windows-gnu", "x86_64-pc-windows-gnullvm"),
        default="x86_64-pc-windows-msvc",
    )
    args = parser.parse_args()
    env = os.environ.copy()
    # Include compiler runtimes, including LLVM's unwinder, in the executables.
    # Recipients should not need Visual C++ or MinGW runtime DLL installations.
    flags = env.get("CARGO_ENCODED_RUSTFLAGS")
    if flags is None:
        flags = "\x1f".join(shlex.split(env.get("RUSTFLAGS", "")))
    env["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join(
        filter(None, [flags, "-C", "target-feature=+crt-static"])
    )
    build = ROOT / "target" / "windows-package"
    subprocess.run(
        ["cargo", "build", "--release", "--locked", "--bins", "--features", "gui",
         "--target", args.target, "--target-dir", str(build)],
        cwd=ROOT, env=env, check=True,
    )
    dist = ROOT / "dist"
    package = dist / "codesync-windows-x64"
    package.mkdir(parents=True, exist_ok=True)
    files = ["codesync-gui.exe", "codesync.exe", "README.txt"]
    for name in files[:2]:
        shutil.copy2(build / args.target / "release" / name, package / name)
    shutil.copy2(ROOT / "packaging" / "windows" / "README.txt", package / files[2])
    archive = dist / "codesync-windows-x64.zip"
    with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as output:
        # Explicit list: never include local profiles, credentials, or stray files.
        for name in files:
            output.write(package / name, f"{package.name}/{name}")
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_suffix(".zip.sha256").write_text(
        f"{digest}  {archive.name}\n", encoding="ascii"
    )
    print(f"Windows app: {package / files[0]}\nDownload: {archive}", flush=True)


if __name__ == "__main__":
    main()
