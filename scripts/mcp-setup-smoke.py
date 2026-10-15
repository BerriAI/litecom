"""Guided setup with real native MCP clients; isolated config, no model calls."""
import argparse
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import socket
import subprocess
import tempfile
import time
import tomllib
from playwright.sync_api import sync_playwright, expect

parser = argparse.ArgumentParser()
parser.add_argument('--agent', choices=['codex', 'claude', 'opencode', 'pi'], required=True)
parser.add_argument('--binary', type=Path, default=Path('target/debug/litecom'))
parser.add_argument('--path', action='append', default=[], help='Additional installed agent bin directory')
args = parser.parse_args()
binary = args.binary.resolve()

with tempfile.TemporaryDirectory(prefix='litecom-mcp-setup-') as folder:
    root = Path(folder)
    work = root/'work'
    work.mkdir()
    env = dict(os.environ, LITECOM_HOME=str(root/'litecom'),
               XDG_CONFIG_HOME=str(root/'config'), XDG_DATA_HOME=str(root/'data'),
               XDG_CACHE_HOME=str(root/'cache'), XDG_STATE_HOME=str(root/'state'),
               OPENCODE_CONFIG_DIR=str(root/'opencode'), PI_CODING_AGENT_DIR=str(root/'pi'),
               CODEX_HOME=str(root/'codex'), CLAUDE_CONFIG_DIR=str(root/'claude'),
               PATH=os.pathsep.join(args.path+[os.environ['PATH']]))
    # A conflicting, disabled server must survive setup and never be launched.
    if args.agent == 'opencode':
        config = root/'config/opencode/opencode.json'
        initial = {'mcp': {'litecom': {'type': 'local', 'command': ['do-not-run'], 'enabled': False}}}
        key = 'mcp'
    elif args.agent == 'pi':
        config = root/'pi/mcp.json'
        initial = {'mcpServers': {'litecom': {'command': 'do-not-run', 'enabled': False}}}
        key = 'mcpServers'
    elif args.agent == 'codex':
        config = root/'codex/config.toml'
        initial = {'mcp_servers': {'litecom': {'command': 'do-not-run', 'enabled': False}}}
        key = 'mcp_servers'
    else:
        config = root/'claude/.claude.json'
        initial = {'mcpServers': {'litecom': {'type': 'stdio', 'command': 'do-not-run', 'args': [], 'env': {}}}}
        key = 'mcpServers'
    config.parent.mkdir(parents=True, exist_ok=True)
    config.write_text('[mcp_servers.litecom]\ncommand = "do-not-run"\nenabled = false\n' if args.agent == 'codex' else json.dumps(initial))

    def read_config():
        return tomllib.loads(config.read_text()) if args.agent == 'codex' else json.loads(config.read_text())
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    origin = f'http://127.0.0.1:{port}'
    log = root/'relay.log'
    processes = []
    master = None
    with log.open('w') as relay_log, sync_playwright() as p:
        browser = p.chromium.launch(channel='chrome', headless=True)
        try:
            relay = subprocess.Popen([binary, 'relay', '--listen', f'127.0.0.1:{port}',
                                      '--public-url', origin, '--data', root/'relay', '--dev'],
                                     env=env, stdout=relay_log, stderr=relay_log, start_new_session=True)
            processes.append(relay)
            page = browser.new_page(viewport={'width': 390, 'height': 844})
            for attempt in range(100):
                try:
                    if page.request.get(origin+'/api/capabilities').json()['development']:
                        break
                except Exception:
                    time.sleep(.1)
            else:
                raise AssertionError('Development relay did not start')
            command = [binary, 'setup', '--relay', origin, '--agent', args.agent,
                       '--workspace', work, '--no-background', '--no-browser']
            master, slave = pty.openpty()
            setup = subprocess.Popen(command, env=env, cwd=work, stdin=slave,
                                     stdout=slave, stderr=slave, start_new_session=True)
            processes.append(setup)
            os.close(slave)
            output = ''

            def until(text):
                global output
                deadline = time.monotonic()+50
                while text not in output:
                    assert time.monotonic() < deadline, output
                    if select.select([master], [], [], .1)[0]:
                        try:
                            output += os.read(master, 65536).decode(errors='replace')
                        except OSError:
                            raise AssertionError(output)
                    elif setup.poll() is not None:
                        raise AssertionError(output)
                return output

            until('Where do you want to approve requests?')
            os.write(master, b'\r')
            until('?approve=phone')
            page.goto(re.search(re.escape(origin)+r'/connect/[^\s]+\?approve=phone', output).group(0))
            page.get_by_label('Your name').fill('MCP setup test')
            page.get_by_label('Phone number').fill('+14155550238')
            page.get_by_role('button', name='Send sign-in code').click()
            expect(page.get_by_label('Sign-in code')).to_be_visible()
            code = re.findall(r'Your Litecom sign-in code is (\d{6})', log.read_text())[-1]
            page.get_by_label('Sign-in code').fill(code)
            page.get_by_role('button', name='Continue', exact=True).click()
            page.get_by_role('button', name='Connect agent', exact=True).click()
            until('Start listening:')
            setup.wait(timeout=10)
            assert setup.returncode == 0, output
            saved = config.read_text()
            servers = read_config()[key]
            assert servers['litecom'] == initial[key]['litecom'], servers
            assert 'litecom_2' in servers, servers
            # Repeated setup should verify/reuse, not rewrite, the native config.
            repeated = subprocess.run(command, env=env, cwd=work, stdin=subprocess.DEVNULL,
                                      capture_output=True, text=True, timeout=50)
            assert repeated.returncode == 0, repeated.stdout+repeated.stderr
            assert config.read_text() == saved
            client_command = [args.agent, 'mcp', 'list']
            if args.agent == 'opencode':
                client_command.insert(1, '--pure')
            elif args.agent == 'claude':
                client_command = ['claude', 'mcp', 'get', 'litecom_2']
            elif args.agent == 'codex':
                client_command = ['codex', 'mcp', 'get', 'litecom_2', '--json']
            checked = subprocess.run(client_command, env=env, cwd=work, stdin=subprocess.DEVNULL,
                                     capture_output=True, text=True, timeout=40)
            assert checked.returncode == 0, checked.stdout+checked.stderr
            plain = re.sub(r'\x1b\[[0-9;]*m', '', checked.stdout+checked.stderr)
            if args.agent == 'codex':
                actual = json.loads(checked.stdout)
                assert actual['enabled'] is True
                assert actual['transport']['command'] == servers['litecom_2']['command']
                assert actual['transport']['args'] == servers['litecom_2']['args']
            elif args.agent == 'claude':
                assert re.search(r'Status: [✓✔] Connected', plain), plain
                assert 'Scope: User config' in plain, plain
            else:
                assert re.search(r'litecom_2[^\n]*connected', plain, re.I), plain

            if args.agent in ('codex', 'claude'):
                # A disabled existing connection must not produce an enabled duplicate.
                if args.agent == 'codex':
                    config.write_text(saved.replace('[mcp_servers.litecom_2]', '[mcp_servers.litecom_2]\nenabled = false'))
                else:
                    disabled = read_config()
                    disabled.setdefault('projects', {}).setdefault(str(work.resolve()), {})['disabledMcpServers'] = ['litecom_2']
                    config.write_text(json.dumps(disabled))
                disabled_servers = read_config()[key]
                blocked = subprocess.run(command, env=env, cwd=work, stdin=subprocess.DEVNULL,
                                         capture_output=True, text=True, timeout=50)
                assert blocked.returncode != 0, blocked.stdout+blocked.stderr
                assert read_config()[key] == disabled_servers

                # Malformed config must be preserved, not treated as an empty config.
                malformed = '{not valid configuration'
                config.write_text(malformed)
                failed = subprocess.run(command, env=env, cwd=work, stdin=subprocess.DEVNULL,
                                        capture_output=True, text=True, timeout=50)
                assert failed.returncode != 0
                assert config.read_text() == malformed
            evidence = 'saved native configuration verified' if args.agent == 'codex' else 'native client connected'
            print(f'PASS: real {args.agent} guided MCP setup, preserved collision, idempotent repeat, {evidence}; no model calls.')
        finally:
            browser.close()
            for process in reversed(processes):
                if process.poll() is None:
                    os.killpg(process.pid, signal.SIGTERM)
                    try:
                        process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid, signal.SIGKILL)
                        process.wait(timeout=10)
            if master is not None:
                os.close(master)
