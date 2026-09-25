"""Verify SSH control using a fake service manager; never change real services."""
import json
import os
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
script = (root / 'src/gui/host-control.sh').read_text()
# Exercise the systemd branch independently of the test machine's init system.
script = script.replace('[ -d /run/systemd/system ]', 'true')
with tempfile.TemporaryDirectory(prefix='codesync-service-test-') as directory:
    temp = Path(directory)
    state_file = temp / 'state.json'
    binary = temp / 'systemctl'
    binary.write_text('''#!/usr/bin/python3
import json, os, sys
from pathlib import Path
p = Path(os.environ['TEST_STATE'])
s = json.loads(p.read_text())
a = sys.argv[1:]
aliases = {'ssh.service': 'ssh.service', 'sshd.service': 'ssh.service', 'ssh.socket': 'ssh.socket'}
if a[0] == 'list-units': sys.exit(0)
if a[0] == 'cat': sys.exit(0 if a[1] in aliases else 1)
if a[0] == 'show': print(aliases[a[-1]]); sys.exit(0)
if a[0] == 'is-active': sys.exit(0 if any(s['active'].get(aliases.get(x, x), False) for x in a[2:]) else 3)
if a[0] == 'disable':
    if os.environ.get('TEST_FAIL') == 'yes': sys.exit(1)
    unit = a[-1]
    s['active'][unit] = False
    s['disabled'].append(unit)
    p.write_text(json.dumps(s))
    sys.exit(0)
sys.exit(2)
''')
    binary.chmod(0o700)
    env = dict(os.environ, PATH=f'{temp}:/usr/bin:/bin', TEST_STATE=str(state_file))
    initial = {'active': {'ssh.service': True, 'ssh.socket': True, 'unrelated.service': True}, 'disabled': []}
    state_file.write_text(json.dumps(initial))
    def run(mode, fail=False):
        return subprocess.run(['sh', '-c', script, 'test', mode], env=dict(env, TEST_FAIL='yes' if fail else 'no'), capture_output=True, text=True)
    assert run('status').stdout.strip() == 'enabled'
    assert run('disable', fail=True).returncode != 0
    assert json.loads(state_file.read_text()) == initial
    assert run('disable').returncode == 0
    state = json.loads(state_file.read_text())
    assert state['active']['unrelated.service']
    assert state['disabled'] == ['ssh.socket', 'ssh.service']
    assert run('status').stdout.strip() == 'disabled'
    assert run('unexpected').returncode == 2
for name in ['host-control.sh', 'host-setup.sh', 'tailscale-setup.sh']:
    subprocess.run(['sh', '-n', str(root / 'src/gui' / name)], check=True)
print('SSH control checks passed: socket and service disable, aliases, failure handling, status, and shell syntax.')
