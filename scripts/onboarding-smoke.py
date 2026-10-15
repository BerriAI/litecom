"""Real terminal/browser/worker flow. Uses a fake native agent and a --dev relay."""
import argparse
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import subprocess
import tempfile
import time
from playwright.sync_api import sync_playwright, expect

parser = argparse.ArgumentParser()
parser.add_argument('--url', default='http://127.0.0.1:4141')
parser.add_argument('--dev-log', type=Path, required=True)
parser.add_argument('--binary', type=Path, default=Path('target/debug/litecom'))
parser.add_argument('--with-service', action='store_true')
parser.add_argument('--approval', choices=['phone','email'], default='phone')
args = parser.parse_args()
binary = args.binary.resolve()
connection = 'qa-' + str(time.time_ns())
screenshots = Path('.preview/onboarding')
screenshots.mkdir(parents=True, exist_ok=True)

class Terminal:
    def __init__(self, command, env, cwd):
        self.master, slave = pty.openpty()
        self.process = subprocess.Popen(command, stdin=slave, stdout=slave, stderr=slave, env=env, cwd=cwd, start_new_session=True)
        os.close(slave)
        self.output = ''

    def until(self, text, timeout=40):
        deadline = time.monotonic() + timeout
        while text not in self.output:
            if time.monotonic() > deadline:
                raise AssertionError(f'Terminal did not reach {text!r}:\n{self.output}')
            if select.select([self.master], [], [], 0.2)[0]:
                try:
                    self.output += os.read(self.master, 65536).decode(errors='replace')
                except OSError:
                    raise AssertionError(f'Terminal exited before {text!r}:\n{self.output}')
        return self.output

    def enter(self):
        os.write(self.master, b'\r')

    def finish(self):
        self.process.wait(timeout=20)
        assert self.process.returncode == 0, self.output

    def close(self):
        if self.process.poll() is None:
            os.killpg(self.process.pid, signal.SIGTERM)
            self.process.wait(timeout=10)
        os.close(self.master)

with tempfile.TemporaryDirectory(prefix='litecom-onboarding-') as directory:
    root = Path(directory)
    work = root/'work'
    work.mkdir()
    programs = root/'bin'
    programs.mkdir()
    fake = programs/'codex'
    fake.write_text('''#!/bin/sh
cat >/dev/null
while [ "$#" -gt 0 ]; do
  if [ "$1" = --output-last-message ]; then shift; cp reply.fixture "$1"; fi
  shift
done
''')
    fake.chmod(0o700)
    attachment = work/'notes.txt'
    attachment.write_bytes(b'Prepared by the deterministic test agent.\n')
    (work/'reply.fixture').write_text(json.dumps({'text':'The requested notes are attached.', 'files':[str(attachment)]}))
    env = dict(os.environ, LITECOM_HOME=str(root/'config'), PATH=str(programs)+os.pathsep+os.environ['PATH'])
    prefix = [str(binary), '--connection', connection]
    command = prefix+['setup', '--relay', args.url, '--no-mcp', '--no-browser']
    if not args.with_service:
        command.append('--no-background')
    terminal = worker = None
    with sync_playwright() as p:
        browser = p.chromium.launch(channel='chrome', headless=True)
        try:
            page = browser.new_page(viewport={'width':390,'height':844}, accept_downloads=True)
            assert page.request.get(args.url+'/api/capabilities').json()['development'], 'Requires explicit --dev delivery.'
            terminal = Terminal(command, env, work)
            terminal.until('Where do you want to approve requests?')
            if args.approval=='email':
                os.write(terminal.master,b'\x1b[B\x1b[B')
            terminal.enter()
            output = terminal.until('?approve='+args.approval)
            url = re.search(re.escape(args.url)+r'/connect/[A-Za-z0-9-]+\?approve='+args.approval,output).group(0)
            page.goto(url)
            expect(page.get_by_text('Approve requests on '+args.approval+'. You can add channels later.')).to_be_visible()
            expect(page.get_by_role('radio')).to_have_count(0)
            phone = '+1202'+str(int(time.time()))[-6:]+'7'
            page.get_by_label('Your name').fill('Onboarding test')
            page.get_by_label('Phone number').fill(phone)
            page.get_by_role('button',name='Send sign-in code').click()
            expect(page.get_by_label('Sign-in code')).to_be_visible()
            code = re.findall(re.escape(phone)+r': Your Litecom sign-in code is (\d{6})', args.dev_log.read_text())[-1]
            page.get_by_label('Sign-in code').fill(code)
            page.get_by_role('button',name='Continue',exact=True).click()
            if args.approval=='email':
                expect(page.get_by_label('Email address')).to_be_visible()
                email=connection+'@example.test'
                page.get_by_label('Email address').fill(email)
                page.get_by_role('button',name='Send email code').click()
                expect(page.get_by_label('Email code')).to_be_visible()
                email_code=re.findall(re.escape(email)+r': Your Litecom email code is (\d{6})',args.dev_log.read_text())[-1]
                page.reload()
                expect(page.get_by_label('Email code')).to_be_visible()
                page.get_by_label('Email code').fill(email_code)
                page.get_by_role('button',name='Connect email',exact=True).click()
            expect(page.get_by_role('heading',name='Connect this agent?')).to_be_visible()
            page.get_by_role('button',name='Connect agent',exact=True).click()
            terminal.until('Which agent should handle your requests?')
            terminal.enter()
            terminal.until('Folder your agent can work in')
            terminal.enter()
            terminal.until('Add a person:')
            terminal.finish()
            terminal.close()
            terminal = None
            repeated = subprocess.run(command,env=env,cwd=work,capture_output=True,text=True,timeout=40)
            assert repeated.returncode == 0, repeated.stderr
            assert 'Where do you want' not in repeated.stdout and 'Which agent' not in repeated.stdout
            if args.approval=='phone':
                # A signed-in browser must still honor the newly selected destination.
                before=page.request.get(args.url+'/api/me').json()['id']
                device=page.request.post(args.url+'/device/start',data={'label':'Returning connection fixture'}).json()
                page.goto(args.url+'/connect/'+device['user_code']+'?approve=email')
                expect(page.get_by_label('Email address')).to_be_visible()
                email=connection+'@example.test'
                page.get_by_label('Email address').fill(email)
                page.get_by_role('button',name='Send email code').click()
                expect(page.get_by_label('Email code')).to_be_visible()
                email_code=re.findall(re.escape(email)+r': Your Litecom email code is (\d{6})',args.dev_log.read_text())[-1]
                page.get_by_label('Email code').fill(email_code)
                page.get_by_role('button',name='Connect email',exact=True).click()
                expect(page.get_by_role('heading',name='Connect this agent?')).to_be_visible()
                assert page.request.get(args.url+'/api/me').json()['id']==before
                page.goto(args.url+'/settings')
                expect(page.get_by_role('button',name='Turn off Email',exact=True)).to_be_visible()
                page.get_by_role('button',name='Turn off Email',exact=True).click()
                expect(page.get_by_role('button',name='Connect Email',exact=True)).to_be_visible()
                expect(page.get_by_role('button',name='Turn off Phone',exact=True)).to_be_visible()
            if not args.with_service:
                worker = subprocess.Popen(prefix+['worker'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,start_new_session=True)
            asked = subprocess.run(prefix+['ask','me','Find the notes and attach them.'],env=env,capture_output=True,text=True,timeout=20)
            assert asked.returncode == 0, asked.stderr
            request_url = re.search(re.escape(args.url)+r'/request/[a-f0-9-]+',asked.stdout).group(0)
            page.goto(request_url)
            expect(page.get_by_role('button',name='Approve run',exact=True)).to_be_visible(timeout=15000)
            assert page.evaluate('document.documentElement.scrollWidth <= innerWidth'), 'Run approval overflow'
            page.screenshot(path=str(screenshots/'approve-run-mobile.png'),full_page=True)
            page.get_by_role('button',name='Approve run',exact=True).click()
            expect(page.get_by_role('button',name='Approve',exact=True)).to_be_visible(timeout=20000)
            expect(page.locator('main')).to_contain_text('The requested notes are attached.')
            page.screenshot(path=str(screenshots/'approve-result-mobile.png'),full_page=True)
            page.get_by_role('button',name='Approve',exact=True).click()
            expect(page.get_by_role('button',name='Revoke access')).to_be_visible()
            with page.expect_download() as download:
                page.get_by_role('link',name='notes.txt',exact=True).click()
            assert Path(download.value.path()).read_bytes()==attachment.read_bytes()
            expect(page.locator('main')).to_contain_text('1 file download verified',timeout=10000)
            assert page.evaluate('document.documentElement.scrollWidth <= innerWidth')
            page.goto(args.url+'/settings')
            page.get_by_role('button',name='Disconnect',exact=True).click()
            terminal = Terminal(command, env, work)
            terminal.until("Let's reconnect your phone account.")
            terminal.until('Where do you want to approve requests?')
            terminal.close()
            terminal = None
            assert (root/'config'/'connections'/f'{connection}.json').exists()
            print('PASS: phone-first terminal setup, one channel choice, native detection, default folder, repeat setup, self request, launch approval, sharing approval, verified download, revoked-connection recovery; fake agent and local delivery.')
            print('Measured setup: 3 terminal confirmations + '+('5 browser buttons + 5 typed fields' if args.approval=='email' else '3 browser buttons + 3 typed fields')+'. Installation, server address, code reading and selection navigation are additional prerequisites/work.')
            if args.with_service:
                print('PASS: actual macOS background startup and queue heartbeat; temporary service removed in cleanup.')
        except Exception:
            if terminal:
                print(terminal.output)
            page.screenshot(path=str(screenshots/'failure.png'),full_page=True)
            print(page.locator('main').inner_text())
            raise
        finally:
            if terminal:
                terminal.close()
            if worker:
                os.killpg(worker.pid,signal.SIGTERM)
                worker.wait(timeout=10)
            if args.with_service:
                cleanup=subprocess.run(prefix+['service','remove'],env=env,capture_output=True,text=True,timeout=20)
                assert cleanup.returncode==0,cleanup.stderr
            browser.close()
