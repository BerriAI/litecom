"""Exercise the README's one-command preview with a deterministic native-agent fixture."""
import argparse
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import subprocess
import sys
import tempfile
import time
from playwright.sync_api import sync_playwright, expect

parser = argparse.ArgumentParser()
parser.add_argument('--binary', type=Path, default=Path('target/debug/litecom'))
args = parser.parse_args()
binary = args.binary.resolve()
screenshots = Path('.preview/preview')
screenshots.mkdir(parents=True, exist_ok=True)

with tempfile.TemporaryDirectory(prefix='litecom-preview-qa-') as directory:
    root = Path(directory)
    programs = root/'bin'
    programs.mkdir()
    fake = programs/'codex'
    fake.write_text(f'#!{sys.executable}\n' + '''import json, os, sys
from pathlib import Path
assert sys.argv[1] == 'exec', 'Preview must not edit native MCP configuration'
sys.stdin.read()
with Path(os.environ['LITECOM_QA_CALLS']).open('a') as f:
    f.write(json.dumps({'cwd': str(Path.cwd()), 'argv': sys.argv})+'\\n')
output = Path(sys.argv[sys.argv.index('--output-last-message')+1])
output.write_text(json.dumps({'text':'Here is the sample file you requested.', 'files':[str(Path.cwd()/'welcome.txt')]}))
''')
    fake.chmod(0o700)
    calls = root/'calls.jsonl'
    env = dict(os.environ, PATH=str(programs)+os.pathsep+os.environ['PATH'], LITECOM_QA_CALLS=str(calls))
    master, slave = pty.openpty()
    proc = subprocess.Popen([str(binary), 'try', '--no-browser'], stdin=slave, stdout=slave, stderr=slave, env=env, start_new_session=True)
    os.close(slave)
    output = ''

    def until(text, timeout=40):
        global output
        deadline = time.monotonic()+timeout
        while text not in output:
            if time.monotonic()>deadline:
                raise AssertionError(f'Did not reach {text!r}:\n{output}')
            if select.select([master], [], [], 0.2)[0]:
                try:
                    output += os.read(master,65536).decode(errors='replace')
                except OSError:
                    raise AssertionError(f'Preview exited before {text!r}:\n{output}')
        return output

    with sync_playwright() as p:
        browser = p.chromium.launch(channel='chrome', headless=True)
        page = browser.new_page(viewport={'width':390,'height':844}, accept_downloads=True)
        try:
            until('Where do you want to approve requests?')
            os.write(master,b'\r')
            until('?approve=phone')
            connect = re.search(r'http://127\.0\.0\.1:\d+/connect/[A-Za-z0-9-]+\?approve=phone',output).group(0)
            origin = connect.split('/connect/')[0]
            page.goto(connect)
            page.get_by_label('Your name').fill('Preview test')
            page.get_by_label('Phone number').fill('+14155550123')
            page.get_by_role('button',name='Send sign-in code').click()
            until('Your Litecom sign-in code is ')
            code = re.search(r'Your Litecom sign-in code is (\d{6})',output).group(1)
            page.get_by_label('Sign-in code').fill(code)
            page.get_by_role('button',name='Continue',exact=True).click()
            page.get_by_role('button',name='Connect agent',exact=True).click()
            until('Which agent should handle your requests?')
            os.write(master,b'\r')
            until('Your first request is ready.')
            request = re.search(r'http://127\.0\.0\.1:\d+/request/[a-f0-9-]+',output).group(0)
            page.goto(request)
            expect(page.get_by_role('button',name='Approve run',exact=True)).to_be_visible(timeout=15000)
            task_id = request.rsplit('/',1)[1]
            task = page.request.get(origin+'/api/tasks/'+task_id).json()
            workspace = Path(task['draft']['action']['workspace'])
            assert workspace.name=='example'
            assert not calls.exists(), 'Agent ran without approval'
            assert page.evaluate('document.documentElement.scrollWidth <= innerWidth')
            page.screenshot(path=str(screenshots/'run-mobile.png'),full_page=True)
            page.get_by_role('button',name='Approve run',exact=True).click()
            expect(page.get_by_role('button',name='Approve',exact=True)).to_be_visible(timeout=20000)
            assert len(calls.read_text().splitlines())==1
            assert page.request.get(origin+'/api/tasks/'+task_id).json()['response'] is None
            page.get_by_role('button',name='Approve',exact=True).click()
            expect(page.get_by_role('button',name='Revoke access')).to_be_visible()
            with page.expect_download() as download:
                page.get_by_role('link',name='welcome.txt',exact=True).click()
            assert Path(download.value.path()).read_bytes()==(workspace/'welcome.txt').read_bytes()
            expect(page.locator('main')).to_contain_text('1 file download verified',timeout=10000)
            page.screenshot(path=str(screenshots/'result-mobile.png'),full_page=True)
            page.goto(origin+'/agents')
            page.locator('#devin-connect summary').first.click()
            expect(page.get_by_label('Devin token')).to_be_visible()
            assert page.evaluate('document.documentElement.scrollWidth <= innerWidth')
            page.screenshot(path=str(screenshots/'agents-mobile.png'),full_page=True)
            page.set_viewport_size({'width':1100,'height':900})
            page.screenshot(path=str(screenshots/'agents-desktop.png'),full_page=True)
            os.kill(proc.pid,signal.SIGINT)
            proc.wait(timeout=15)
            assert proc.returncode==0, output
            assert not workspace.parent.exists(), 'Temporary preview data was not removed'
            print('PASS: one-command preview, Phone choice, browser pairing, agent choice, two approvals, exact download, no MCP edits, clean exit. Native agent and delivery are fixtures.')
            print('Measured preview: 1 command, 2 terminal selections, 3 sign-in/connect buttons, 3 fields; then 2 approvals and 1 download. Build/install and existing agent login are prerequisites.')
        except Exception:
            print(output)
            print(page.locator('main').inner_text())
            page.screenshot(path=str(screenshots/'failure.png'),full_page=True)
            raise
        finally:
            if proc.poll() is None:
                os.killpg(proc.pid,signal.SIGTERM)
                proc.wait(timeout=15)
            os.close(master)
            browser.close()
