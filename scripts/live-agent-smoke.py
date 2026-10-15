"""Live native-agent test. Uses the agent's account and normal model billing.

Creates two local test accounts, uses a real native/ACP process, and verifies
project instructions, a local file operation, sharing approval, and exact bytes.
No messages are sent and no native MCP configuration or startup service is added.
Requires an installed, signed-in agent, Python 3, Playwright, and Chrome.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import pty
import re
import select
import shutil
import signal
import socket
import subprocess
import tempfile
import time
import uuid
from playwright.sync_api import sync_playwright, expect

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--agent', choices=['codex', 'claude', 'codex-acp', 'opencode'], required=True)
parser.add_argument('--path', type=Path, help='Prepend an isolated agent installation to PATH')
parser.add_argument('--review-permissions', action='store_true', help='For Codex ACP: start read-only, save permission proposals, and wait for a .decision file containing approve or decline')
parser.add_argument('--binary', type=Path, default=Path('target/debug/litecom'))
parser.add_argument('--timeout', type=int, default=300, help='Test deadline in seconds; does not set model token limits')
args = parser.parse_args()
if args.review_permissions and args.agent != 'codex-acp':
    parser.error('--review-permissions requires --agent codex-acp')
agent_env = dict(os.environ)
if args.path:
    agent_env['PATH'] = str(args.path.resolve(strict=True)) + os.pathsep + agent_env['PATH']
if args.review_permissions:
    # Make the test require an actual ACP permission callback. This only affects
    # the test process; it does not change the user's saved agent configuration.
    agent_env['INITIAL_AGENT_MODE'] = 'read-only'
binary = args.binary.resolve()
version = subprocess.check_output([args.agent, '--version'], text=True, env=agent_env).strip()
evidence = Path('.preview/live-agent') / (args.agent + '-' + uuid.uuid4().hex[:8])
evidence.mkdir(parents=True, mode=0o700)
evidence = evidence.resolve()
print(f'LIVE: {version}; normal account usage; local test accounts and sample files only.', flush=True)


def stop(process):
    if process is not None and process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=15)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=10)


with tempfile.TemporaryDirectory(prefix='litecom-live-') as folder:
    root = Path(folder)
    work = root / 'work'
    work.mkdir()
    marker = 'context-' + uuid.uuid4().hex
    numbers = [int(uuid.uuid4().hex[:4], 16) for _ in range(4)]
    (work / 'numbers.json').write_text(json.dumps(numbers))
    instructions = (
        '# Local integration test\n\n'
        f'Include the exact marker `{marker}` in the final reply text. '
        'Do not put it in the result file. Use only files in this folder and local tools. '
        'Do not access external services, use other agents, or attach instruction files.\n'
    )
    (work / 'AGENTS.md').write_text(instructions)
    (work / 'CLAUDE.md').write_text(instructions)
    env = dict(agent_env, LITECOM_HOME=str(root / 'config'))
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    origin = f'http://127.0.0.1:{port}'
    relay = worker = terminal = None
    master = None
    output = ''
    task_url = None
    started = None
    passed = False
    permissions_seen = set()
    permissions_reviewed = set()
    with (root / 'relay.log').open('w') as relay_log, (root / 'worker.log').open('w') as worker_log, sync_playwright() as p:
        browser = p.chromium.launch(channel='chrome', headless=True)
        owner = browser.new_page(viewport={'width': 390, 'height': 844}, accept_downloads=True)
        requester = browser.new_page(viewport={'width': 1100, 'height': 850}, accept_downloads=True)
        try:
            relay = subprocess.Popen([binary, 'relay', '--listen', f'127.0.0.1:{port}',
                                      '--public-url', origin, '--data', root / 'relay', '--dev'],
                                     env=env, stdout=relay_log, stderr=relay_log, start_new_session=True)
            for _ in range(100):
                try:
                    if owner.request.get(origin + '/api/capabilities').json()['development']:
                        break
                except Exception:
                    time.sleep(.1)
            else:
                raise AssertionError('Development relay did not start')

            def sign_in(page, name, phone):
                page.get_by_label('Your name').fill(name)
                page.get_by_label('Phone number').fill(phone)
                page.get_by_role('button', name='Send sign-in code').click()
                expect(page.get_by_label('Sign-in code')).to_be_visible()
                code = re.findall(re.escape(phone) + r': Your Litecom sign-in code is (\d{6})',
                                  (root / 'relay.log').read_text())[-1]
                page.get_by_label('Sign-in code').fill(code)
                page.get_by_role('button', name='Continue', exact=True).click()

            master, slave = pty.openpty()
            terminal = subprocess.Popen([binary, 'setup', '--relay', origin, '--agent', args.agent,
                                         '--workspace', work, '--no-mcp', '--no-background', '--no-browser'],
                                        env=env, cwd=work, stdin=slave, stdout=slave, stderr=slave,
                                        start_new_session=True)
            os.close(slave)

            def until(text):
                global output
                deadline = time.monotonic() + 45
                while text not in output:
                    assert time.monotonic() < deadline, output
                    if select.select([master], [], [], .1)[0]:
                        try:
                            output += os.read(master, 65536).decode(errors='replace')
                        except OSError:
                            raise AssertionError(output)
                    elif terminal.poll() is not None:
                        raise AssertionError(output)

            until('Where do you want to approve requests?')
            os.write(master, b'\r')
            until('?approve=phone')
            owner.goto(re.search(re.escape(origin) + r'/connect/[^\s]+\?approve=phone', output).group(0))
            sign_in(owner, 'Live test owner', '+14155550121')
            owner.get_by_role('button', name='Connect agent', exact=True).click()
            until('Start listening:')
            terminal.wait(timeout=15)
            assert terminal.returncode == 0, output
            os.close(master)
            master = None
            worker = subprocess.Popen([binary, 'worker'], env=env, cwd=work,
                                      stdout=worker_log, stderr=worker_log, start_new_session=True)

            requester.goto(origin)
            sign_in(requester, 'Live test requester', '+14155550122')
            expect(requester.get_by_role('heading', name='Your inbox')).to_be_visible()
            owner.goto(origin + '/contacts')
            requester.goto(owner.get_by_label('Your contact link').input_value())
            requester.get_by_role('button', name='Send contact request').click()
            owner.reload()
            owner.get_by_role('button', name='Accept contact').click()
            expect(owner.get_by_role('button', name='Block')).to_be_visible()
            requester.goto(origin)
            requester.get_by_text('Ask an agent', exact=True).click()
            requester.get_by_label(re.compile('Who')).select_option(label='Live test owner')
            requester.get_by_label('What do you need?').fill(
                'Read numbers.json in the current working folder. Write result.txt containing '
                'only the decimal sum of its numbers followed by one newline. Use local Python '
                'to verify the file contains the correct sum. Attach only result.txt and briefly '
                'report what you did. Follow the project instructions. Work only in this folder; '
                'do not use external services, network tools, or other agents.'
            )
            requester.get_by_role('button', name='Send request', exact=True).click()
            requester.wait_for_url('**/request/*')
            task_url = requester.url
            task_id = task_url.rsplit('/', 1)[1]
            owner.goto(task_url)
            expect(owner.get_by_role('button', name='Approve run', exact=True)).to_be_visible(timeout=15000)
            assert not (work / 'result.txt').exists(), 'Agent changed files before run approval'
            owner.screenshot(path=str(evidence / 'run-mobile.png'), full_page=True)
            started = time.monotonic()
            owner.get_by_role('button', name='Approve run', exact=True).click()
            print('Run approved; waiting for the real agent.', flush=True)
            previous = None
            while time.monotonic() - started < args.timeout:
                task = owner.request.get(origin + '/api/tasks/' + task_id).json()
                if task['status'] != previous:
                    print('Task status:', task['status'], flush=True)
                    previous = task['status']
                for permission in task.get('permissions', []):
                    if permission.get('decision') is not None:
                        continue
                    permission_id = permission['id']
                    if permission_id not in permissions_seen:
                        if args.review_permissions and not permissions_seen:
                            assert not (work / 'result.txt').exists(), 'Agent wrote the result before tool approval'
                        hidden = requester.request.get(origin + '/api/tasks/' + task_id).json()
                        assert not hidden.get('permissions'), 'Private tool permissions were exposed to the requester'
                        proposal = evidence / f'permission-{permission_id}.json'
                        proposal.write_text(json.dumps(permission, indent=2))
                        proposal.chmod(0o600)
                        expect(owner.locator(f'button[data-action="approve-permission"][data-id="{permission_id}"]')).to_be_visible(timeout=15000)
                        owner.screenshot(path=str(evidence / f'permission-{permission_id}.png'), full_page=True)
                        print(f'Review permission: {proposal}\nWrite approve or decline to {proposal.with_suffix(".decision")}', flush=True)
                        permissions_seen.add(permission_id)
                    decision_path = evidence / f'permission-{permission_id}.decision'
                    if decision_path.exists() and permission_id not in permissions_reviewed:
                        decision = decision_path.read_text().strip()
                        assert decision in ('approve', 'decline'), 'Permission decision must be approve or decline'
                        action = 'approve-permission' if decision == 'approve' else 'decline-permission'
                        owner.locator(f'button[data-action="{action}"][data-id="{permission_id}"]').click()
                        permissions_reviewed.add(permission_id)
                if task['status'] == 'awaiting_approval' and not task['draft'].get('action'):
                    break
                if task['status'] in ('failed', 'cancelled', 'expired', 'completed'):
                    raise AssertionError('Agent did not produce a reviewable result: ' + task['status'])
                time.sleep(1)
            else:
                raise AssertionError('Live agent exceeded the test deadline; it will be stopped, not retried')
            (evidence / 'review.json').write_text(json.dumps(task, indent=2))
            (evidence / 'review.json').chmod(0o600)
            if args.review_permissions:
                assert permissions_reviewed, 'Agent finished without the required ACP permission review'
            assert marker in task['draft']['text'], 'Native project instructions were not reflected in the reply'
            expected = (str(sum(numbers)) + '\n').encode()
            assert (work / 'result.txt').read_bytes() == expected, 'Actual file operation produced incorrect bytes'
            hidden = requester.request.get(origin + '/api/tasks/' + task_id).json()
            assert hidden['response'] is None and hidden['draft'] is None
            expect(owner.get_by_role('button', name='Approve', exact=True)).to_be_visible(timeout=15000)
            owner.screenshot(path=str(evidence / 'review-mobile.png'), full_page=True)
            owner.get_by_role('button', name='Approve', exact=True).click()
            expect(requester.get_by_role('link', name='result.txt', exact=True)).to_be_visible(timeout=15000)
            with requester.expect_download() as download:
                requester.get_by_role('link', name='result.txt', exact=True).click()
            assert Path(download.value.path()).read_bytes() == expected
            requester.get_by_role('button', name='Mark received', exact=True).click()
            expect(requester.locator('main')).to_contain_text('Received.')
            requester.screenshot(path=str(evidence / 'received-desktop.png'), full_page=True)
            summary = {'agent': args.agent, 'version': version, 'elapsed_seconds': round(time.monotonic() - started, 1),
                       'sha256': hashlib.sha256(expected).hexdigest(), 'project_instructions': True,
                       'actual_file_operation': True, 'two_account_transfer': True,
                       'tool_permissions_reviewed': len(permissions_reviewed), 'delivery': 'local development'}
            (evidence / 'summary.json').write_text(json.dumps(summary, indent=2))
            (evidence / 'summary.json').chmod(0o600)
            passed = True
            print('PASS: real native agent, project instructions, verified local file operation, separate sharing approval, two-account download and receipt.', flush=True)
        finally:
            if task_url and not passed:
                # Cancel through the owner surface before terminating the local worker.
                try:
                    owner.goto(task_url)
                    owner.get_by_role('button', name='Cancel request', exact=True).click(timeout=3000)
                except Exception:
                    pass
            stop(worker)
            stop(terminal)
            stop(relay)
            if not passed:
                for path in (root / 'config' / 'runs').glob('*'):
                    shutil.copytree(path, evidence / path.name)
                shutil.copyfile(root / 'worker.log', evidence / 'worker.log')
                (evidence / 'worker.log').chmod(0o600)
                print(f'Private failure diagnostics: {evidence}', flush=True)
            if master is not None:
                os.close(master)
            browser.close()
print(f'Evidence: {evidence}', flush=True)
