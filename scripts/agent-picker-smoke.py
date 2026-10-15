"""Terminal selection, piped stdin, retries and detection without model calls."""
import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import threading
from terminal_qa import run_terminal

parser = argparse.ArgumentParser()
parser.add_argument('--binary', type=Path, required=True)
binary = parser.parse_args().binary.resolve()
polls = []


class Relay(BaseHTTPRequestHandler):
    def do_GET(self):
        self.reply({'name': 'Picker test'})

    def do_POST(self):
        if self.path == '/device/poll':
            polls.append(True)
            self.reply({'token': 'test-token', 'user_id': 'test-user', 'agent_id': 'test-agent', 'expires_at': 9999999999})
        else:
            self.reply({})

    def reply(self, value):
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.end_headers()
        self.wfile.write(json.dumps(value).encode())

    def log_message(self, *args):
        pass


server = ThreadingHTTPServer(('127.0.0.1', 0), Relay)
threading.Thread(target=server.serve_forever, daemon=True).start()
try:
    with tempfile.TemporaryDirectory(prefix='litecom-picker-') as folder:
        root = Path(folder)
        programs = root/'bin'
        programs.mkdir()
        env = dict(os.environ, LITECOM_HOME=str(root/'config'), PATH=str(programs), TERM='xterm')
        for name in ['codex', 'claude', 'opencode', 'pi', 'pi-acp', 'openclaw', 'hermes', 'cursor-agent', 'codex-acp', 'claude-agent-acp']:
            path = programs/name
            path.write_text('#!/bin/sh\nexit 91\n')
            path.chmod(0o700)
        prefix = [str(binary), 'setup', '--relay', f'http://127.0.0.1:{server.server_port}', '--connect-code', 'test', '--no-mcp', '--no-background']
        # Reproduce curl | sh with piped stdin and a real controlling tty.
        command = 'printf "" | '+shlex.join(prefix)
        result = run_terminal(command, env, root, [('Which agent should handle your requests?', b'\x1b[B\r')])
        assert result.returncode == 0, result.stdout
        saved = root/'config/connections/default.runner.json'
        assert json.loads(saved.read_text())['agent'] == 'claude'
        for label in ['Codex', 'Claude Code', 'OpenCode', 'pi', 'OpenClaw', 'Hermes', 'Cursor', 'Codex (ACP)', 'Claude Agent (ACP)']:
            assert label in result.stdout, result.stdout
        before = saved.read_bytes()
        result = subprocess.run(prefix, env=env, cwd=root, stdin=subprocess.DEVNULL, capture_output=True, text=True)
        assert result.returncode == 0, result.stderr
        assert saved.read_bytes() == before and len(polls) == 1
        result = subprocess.run(prefix+['--agent', 'opencode'], env=env, cwd=root, stdin=subprocess.DEVNULL, capture_output=True, text=True)
        assert result.returncode == 0, result.stderr
        assert json.loads(saved.read_text())['agent'] == 'open_code'
        # First setup without a terminal must not consume the pairing code.
        env['LITECOM_HOME'] = str(root/'noninteractive')
        result = subprocess.run(prefix, env=env, cwd=root, stdin=subprocess.DEVNULL, capture_output=True, text=True)
        assert result.returncode != 0 and len(polls) == 1
        assert 'Choose an agent in a terminal' in result.stderr
        for path in programs.iterdir():
            if path.name != 'codex':
                path.unlink()
        result = subprocess.run(prefix, env=env, cwd=root, stdin=subprocess.DEVNULL, capture_output=True, text=True)
        assert result.returncode == 0 and 'Using Codex' in result.stdout
        (programs/'codex').unlink()
        env['LITECOM_HOME'] = str(root/'no-agents')
        result = subprocess.run(prefix, env=env, cwd=root, stdin=subprocess.DEVNULL, capture_output=True, text=True)
        assert result.returncode != 0 and 'No coding agents found' in result.stderr
        assert len(polls) == 2
        print('PASS: every supported agent listed; non-default selection through piped stdin; saved choice reused; explicit override; no-terminal, one-agent and no-agent cases.')
finally:
    server.shutdown()
    server.server_close()
