"""Installer regression checks; fake Cargo and isolated homes only."""
import os
from pathlib import Path
import subprocess
import tempfile

installer = Path(__file__).resolve().parents[1] / 'install.sh'
with tempfile.TemporaryDirectory(prefix='codesync-install-test-') as directory:
    root = Path(directory)
    fakebin = root / 'tools'
    fakebin.mkdir()
    cargo = fakebin / 'cargo'
    cargo.write_text('''#!/bin/sh
if [ "${FAIL_INSTALL:-}" = yes ]; then exit 1; fi
while [ "$#" -gt 0 ]; do
  if [ "$1" = --root ]; then shift; destination=$1; fi
  shift
done
mkdir -p "$destination/bin"
printf '#!/bin/sh\\nexit 0\\n' > "$destination/bin/codesync"
chmod +x "$destination/bin/codesync"
''')
    cargo.chmod(0o700)
    for shell in ['bash', 'zsh', 'fish', 'unset']:
        home = root / shell
        home.mkdir()
        install_root = home / "cargo space'$(touch PWNED)"
        env = dict(os.environ, HOME=str(home), CARGO_INSTALL_ROOT=str(install_root),
                   CARGO_HOME=str(home / 'cargo'), PATH=f'{fakebin}:/usr/bin:/bin',
                   XDG_CONFIG_HOME=str(home / 'config'), ZDOTDIR=str(home / 'zsh'))
        if shell == 'unset':
            env.pop('SHELL', None)
        else:
            env['SHELL'] = f'/bin/{shell}'
        for _ in range(2):
            subprocess.run(['sh', str(installer), '--cli-only'], env=env, cwd=home, check=True, capture_output=True)
        if shell in ('bash', 'unset'):
            profile = home / '.bashrc'
            assert profile.read_text().count('# Codesync:') == 1
            for profile in [profile, home / '.profile']:
                result = subprocess.run(['/bin/bash', '--noprofile', '--norc', '-c',
                    '. "$1"; . "$1"; command -v codesync', 'test', str(profile)],
                    env=env, cwd=home, check=True, capture_output=True, text=True)
                assert result.stdout.strip() == str(install_root / 'bin/codesync')
        elif shell == 'zsh':
            for profile in [home / 'zsh/.zshrc', home / 'zsh/.zprofile']:
                assert profile.read_text().count('# Codesync:') == 1
                subprocess.run(['/bin/bash', '-n', str(profile)], check=True)
        else:
            text = (home / 'config/fish/conf.d/codesync-path.fish').read_text()
            assert text.count('# Codesync PATH') == 1
            assert "\\'" in text
        assert not (home / 'PWNED').exists()
    failed = root / 'failed'
    failed.mkdir()
    env.update(HOME=str(failed), SHELL='/bin/bash', FAIL_INSTALL='yes')
    result = subprocess.run(['sh', str(installer)], env=env, capture_output=True)
    assert result.returncode != 0
    assert list(failed.iterdir()) == []
print('Installer checks passed: persistent PATH, idempotence, quoting, shell configs, and failed-install behavior.')
