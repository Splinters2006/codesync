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
    # Fresh home: a newly opened interactive shell finds the installed binary
    # without inheriting Cargo's bin directory or manually sourcing a file.
    fresh = root / 'fresh'
    fresh.mkdir()
    fresh_env = dict(os.environ, HOME=str(fresh), SHELL='/bin/bash',
                     CARGO_HOME=str(fresh / 'cargo'),
                     CARGO_INSTALL_ROOT=str(fresh / 'cargo'),
                     PATH=f'{fakebin}:/usr/bin:/bin')
    subprocess.run(['sh', str(installer)], env=fresh_env, check=True, capture_output=True)
    result = subprocess.run(['/bin/bash', '--noprofile', '-ic', 'command -v codesync'],
                            env=dict(fresh_env, PATH='/usr/bin:/bin'), cwd='/tmp',
                            check=True, capture_output=True, text=True)
    assert result.stdout.strip() == str(fresh / 'cargo/bin/codesync')

    # Repair an existing Cargo installation without invoking Cargo again.
    (fresh / '.bashrc').unlink()
    subprocess.run(['sh', str(installer), '--path-only'],
                   env=dict(fresh_env, FAIL_INSTALL='yes'), check=True, capture_output=True)
    assert (fresh / '.bashrc').exists()

    # Reuse an existing user bin directory: visible even to the original PATH.
    user_bin = fresh / '.local/bin'
    user_bin.mkdir(parents=True)
    linked_env = dict(fresh_env, PATH=f'{user_bin}:{fakebin}:/usr/bin:/bin')
    subprocess.run(['sh', str(installer), '--path-only'], env=linked_env,
                   check=True, capture_output=True)
    assert (user_bin / 'codesync').is_symlink()
    result = subprocess.run(['/bin/sh', '-c', 'command -v codesync'], env=linked_env,
                            cwd='/tmp', check=True, capture_output=True, text=True)
    assert result.stdout.strip() == str(user_bin / 'codesync')
    # Never replace someone else's command with our launcher.
    (user_bin / 'codesync').unlink()
    (user_bin / 'codesync').write_text('unrelated command')
    subprocess.run(['sh', str(installer), '--path-only'], env=linked_env,
                   check=True, capture_output=True)
    assert (user_bin / 'codesync').read_text() == 'unrelated command'

    failed = root / 'failed'
    failed.mkdir()
    env.update(HOME=str(failed), SHELL='/bin/bash', FAIL_INSTALL='yes')
    result = subprocess.run(['sh', str(installer)], env=env, capture_output=True)
    assert result.returncode != 0
    assert list(failed.iterdir()) == []
print('Installer checks passed: fresh-home command discovery, PATH-only repair, immediate user-bin access, collision protection, idempotence, quoting, and failed-install behavior.')
