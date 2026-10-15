"""Isolated cleanup, failure recovery and data preservation on every platform."""
import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import uuid

parser = argparse.ArgumentParser()
parser.add_argument('--binary', type=Path, required=True)
binary = parser.parse_args().binary.resolve()
revoked = []


class Relay(BaseHTTPRequestHandler):
    def do_POST(self):
        assert self.path == '/auth/logout'
        revoked.append(self.headers['Authorization'])
        self.send_response(204)
        self.end_headers()

    def log_message(self, *args):
        pass


server = ThreadingHTTPServer(('127.0.0.1', 0), Relay)
thread = threading.Thread(target=server.serve_forever, daemon=True)
thread.start()
try:
    with tempfile.TemporaryDirectory(prefix='litecom-uninstall-') as folder:
        root = Path(folder)
        home = root/'client'
        config = root/'claude/.claude.json'
        install = root/'installed'
        receipt = root/'xdg/litecom/litecom-receipt.json'
        executable = 'litecom.exe' if os.name == 'nt' else 'litecom'
        env = dict(os.environ, LITECOM_HOME=str(home), CLAUDE_CONFIG_DIR=str(config.parent),
                   XDG_CONFIG_HOME=str(root/'xdg'))
        for directory in [home/'connections', home/'bin', home/'downloads', home/'relay', config.parent, install, receipt.parent]:
            directory.mkdir(parents=True, exist_ok=True)
        (home/'bin'/executable).write_text('test worker binary')
        (install/executable).write_text('test installed binary')
        receipt.write_text(json.dumps({'source': {'app_name': 'litecom', 'owner': 'BerriAI'},
                                      'binaries': [executable], 'install_prefix': str(install)}))
        keep = [home/'downloads/result.txt', home/'relay/litecom.db', home/'personal.txt', install/'other-tool']
        for path in keep:
            path.write_text('keep these bytes')
        name = 'qa-uninstall-'+uuid.uuid4().hex
        client = home/'connections'/f'{name}.json'
        runner = home/'connections'/f'{name}.runner.json'
        client.write_text(json.dumps({'relay': f'http://127.0.0.1:{server.server_port}', 'token': 'test-token',
                                      'user_id': 'test-user', 'agent_id': 'test-agent'}))
        # Neither the executable nor the working folder still exists.
        runner.write_text(json.dumps({'agent': 'claude', 'executable': str(root/'missing-agent'),
                                      'workspace': str(root/'missing-working-folder')}))
        owned = {'type': 'stdio', 'command': str(home/'bin'/executable),
                 'args': ['--home', str(home.resolve()), '--connection', name, 'mcp']}
        wrong = dict(owned, args=['--home', str(root/'other-home'), '--connection', name, 'mcp'])
        original = {'mcpServers': {'owned': owned, 'another-home': wrong, 'other-tool': {'command': 'keep'}}, 'preference': True}

        def uninstall():
            return subprocess.run([binary, 'uninstall', '--all'], env=env, cwd=root,
                                  capture_output=True, text=True, timeout=60)

        config.write_text('broken config')
        failed = uninstall()
        assert failed.returncode != 0, failed.stdout
        assert config.read_text() == 'broken config'
        assert client.exists() and runner.exists() and (install/executable).exists()
        assert not revoked
        config.write_text(json.dumps(original))
        success = uninstall()
        assert success.returncode == 0, success.stdout+success.stderr
        assert revoked == ['Bearer test-token']
        assert json.loads(config.read_text()) == {'mcpServers': {'another-home': wrong, 'other-tool': {'command': 'keep'}}, 'preference': True}
        assert not client.exists() and not runner.exists()
        assert not (home/'bin'/executable).exists() and not (install/executable).exists() and not receipt.exists()
        for path in keep:
            assert path.read_text() == 'keep these bytes'
        assert uninstall().returncode == 0
        assert revoked == ['Bearer test-token']
        print('PASS: uninstall preserves malformed settings, retries successfully, revokes only its credential, handles missing agent/folder, preserves unrelated data and repeats safely.')
finally:
    server.shutdown()
    server.server_close()
