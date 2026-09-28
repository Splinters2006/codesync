"""Validate the actual package; exercise its GUI askpass pipe on Windows."""
import hashlib
import json
import os
from pathlib import Path
import secrets
import socket
import struct
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[1]
DIST = ROOT / "dist"
PACKAGE = DIST / "codesync-windows-x64"


def check_pe(path, subsystem):
    image = path.read_bytes()
    assert image[:2] == b"MZ", f"Not a Windows executable: {path}"
    pe = struct.unpack_from("<I", image, 0x3C)[0]
    assert image[pe:pe + 4] == b"PE\0\0"
    assert struct.unpack_from("<H", image, pe + 4)[0] == 0x8664, "Not x64"
    assert struct.unpack_from("<H", image, pe + 24 + 68)[0] == subsystem


def check_askpass():
    # A GUI-subsystem executable still needs stdout when OpenSSH pipes its reply.
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen(1)
        listener.settimeout(15)
        token = secrets.token_hex(32)
        env = dict(os.environ, CODESYNC_ASKPASS="1", CODESYNC_AUTH_TOKEN=token,
                   CODESYNC_AUTH_SOCKET=f"127.0.0.1:{listener.getsockname()[1]}")
        process = subprocess.Popen(
            [str(PACKAGE / "codesync-gui.exe"), "Password:"], env=env,
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            creationflags=subprocess.CREATE_NO_WINDOW,
        )
        try:
            connection, _ = listener.accept()
            with connection:
                connection.settimeout(15)
                with connection.makefile("rb") as reader:
                    request = json.loads(reader.readline(65536))
                    assert request["token"] == token
                    assert request["prompt"] == "Password:"
                    connection.sendall(b'{"answer":"package-test-answer"}\n')
            stdout, stderr = process.communicate(timeout=15)
            assert process.returncode == 0, stderr
            assert stdout.strip() == b"package-test-answer", stdout
        finally:
            if process.poll() is None:
                process.kill()
            process.communicate(timeout=15)


def main():
    archive = DIST / "codesync-windows-x64.zip"
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    assert archive.with_suffix(".zip.sha256").read_text().split()[0] == digest
    with zipfile.ZipFile(archive) as package:
        assert set(package.namelist()) == {
            f"codesync-windows-x64/{name}"
            for name in ("codesync-gui.exe", "codesync.exe", "README.txt")
        }
        for name in package.namelist():
            assert package.read(name) == (DIST / name).read_bytes()
    check_pe(PACKAGE / "codesync-gui.exe", 2)  # Windows GUI, no console on launch
    check_pe(PACKAGE / "codesync.exe", 3)  # Console CLI
    if os.name == "nt":
        subprocess.run([str(PACKAGE / "codesync.exe"), "--help"], check=True)
        check_askpass()
        print("Windows package and executable smoke tests passed.")
    else:
        print("Package and PE checks passed; executable smoke tests require Windows.")


if __name__ == "__main__":
    main()
