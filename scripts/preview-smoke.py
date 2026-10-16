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
parser.add_argument('--axe', type=Path, help='Optional axe-core axe.min.js for UI auditing')
args = parser.parse_args()
binary = args.binary.resolve()
screenshots = Path('.preview/preview')
screenshots.mkdir(parents=True, exist_ok=True)

with tempfile.TemporaryDirectory(prefix='litecom-preview-qa-') as directory:
    root = Path(directory)
    programs = root/'bin'
    programs.mkdir()
    fake = programs/'codex'
    protocol = Path(__file__).resolve().parent / 'fixtures/codex_app_server.py'
    fake.write_text(f'#!{sys.executable}\n' + '''import json, os, sys, runpy
from pathlib import Path
def reply():
    with Path(os.environ['LITECOM_QA_CALLS']).open('a') as f:
        f.write(json.dumps({'cwd': str(Path.cwd()), 'argv': sys.argv})+'\\n')
    return {'text':'Here is the sample file you requested.', 'files':[str(Path.cwd()/'welcome.txt')]}
''' + f"runpy.run_path({str(protocol)!r})['serve'](reply)\n")
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
        audits = []
        def audit(name):
            if not args.axe:
                return
            page.evaluate("async () => { await document.fonts.ready; await new Promise(requestAnimationFrame); await new Promise(requestAnimationFrame); }")
            assert page.locator('body > header').count() == 1
            for width in [320, 390, 768, 1100]:
                page.set_viewport_size({'width': width, 'height': 900})
                page.evaluate("async () => { await new Promise(requestAnimationFrame); await new Promise(requestAnimationFrame); }")
                assert page.evaluate('document.documentElement.scrollWidth <= innerWidth'), (name, width)
            page.set_viewport_size({'width': 320, 'height': 900})
            page.evaluate("document.documentElement.style.fontSize = '200%'")
            assert page.evaluate('document.documentElement.scrollWidth <= innerWidth'), (name, '200%')
            page.evaluate("document.documentElement.style.fontSize = ''")
            page.set_viewport_size({'width': 390, 'height': 844})
            page.evaluate(args.axe.read_text())
            result = page.evaluate("async () => await axe.run(document, {runOnly: {type: 'tag', values: ['wcag2a','wcag2aa','wcag21aa']}})")
            audits.append({'screen': name, 'violations': result['violations'], 'passes': len(result['passes'])})
            (screenshots/'accessibility.json').write_text(json.dumps(audits, indent=2))
            assert not result['violations'], [(v['id'], v['help'], [n['target'] for n in v['nodes']]) for v in result['violations']]
            page.locator('.skip-link').focus()
            page.keyboard.press('Enter')
            assert page.evaluate("document.activeElement.id === 'app'")
            page.keyboard.press('Tab')
            assert page.evaluate("document.querySelector('main').contains(document.activeElement)")
            summaries = page.locator('main details > summary:visible')
            if summaries.count():
                summary = summaries.first
                was_open = summary.evaluate('(el) => el.parentElement.open')
                summary.focus()
                page.keyboard.press('Enter')
                assert summary.evaluate('(el) => el.parentElement.open') != was_open
                page.keyboard.press('Enter')
                assert summary.evaluate('(el) => el.parentElement.open') == was_open
            buttons = page.locator('main button:visible')
            if buttons.count():
                buttons.first.focus()
                assert buttons.first.evaluate('(el) => parseFloat(getComputedStyle(el).outlineWidth) > 0')
            page.locator('main').focus()
            page.screenshot(path=str(screenshots/(name+'-audit.png')), full_page=True)

        try:
            until('Where do you want to approve requests?')
            os.write(master,b'\r')
            until('Which agent should handle your requests?')
            os.write(master,b'\r')
            until('?approve=phone')
            connect = re.search(r'http://127\.0\.0\.1:\d+/connect/[A-Za-z0-9-]+\?approve=phone',output).group(0)
            origin = connect.split('/connect/')[0]
            page.goto(connect)
            expect(page.get_by_label('Your name')).to_be_visible()
            audit('login')
            page.get_by_label('Your name').fill('Preview test')
            page.get_by_label('Phone number').fill('+14155550123')
            page.get_by_role('button',name='Send sign-in code').click()
            until('Your Litecom sign-in code is ')
            code = re.search(r'Your Litecom sign-in code is (\d{6})',output).group(1)
            page.get_by_label('Sign-in code').fill(code)
            page.get_by_role('button',name='Continue',exact=True).click()
            page.get_by_role('button',name='Connect agent',exact=True).click()

            until('Your first request is ready.')
            request = re.search(r'http://127\.0\.0\.1:\d+/request/[a-f0-9-]+',output).group(0)
            page.goto(request)
            expect(page.locator('button[data-action="approve"]')).to_be_visible(timeout=15000)
            task_id = request.rsplit('/',1)[1]
            task = page.request.get(origin+'/api/tasks/'+task_id).json()
            workspace = Path(task['draft']['action']['workspace'])
            assert workspace.name=='example'
            assert not calls.exists(), 'Agent ran without approval'
            assert page.evaluate('document.documentElement.scrollWidth <= innerWidth')
            audit('request')
            page.screenshot(path=str(screenshots/'run-mobile.png'),full_page=True)
            if args.axe:
                page.locator('button[data-action="approve"]').focus()
                page.keyboard.press('Enter')
            else:
                page.locator('button[data-action="approve"]').click()
            expect(page.get_by_role('button',name='Revoke access',exact=True)).to_be_visible(timeout=20000)
            assert len(calls.read_text().splitlines())==1
            assert page.request.get(origin+'/api/tasks/'+task_id).json()['response'] is not None
            assert page.locator('button[data-action="approve"]').count()==0
            expect(page.get_by_role('button',name='Revoke access')).to_be_visible()
            with page.expect_download() as download:
                page.get_by_role('link',name='welcome.txt',exact=True).click()
            assert Path(download.value.path()).read_bytes()==(workspace/'welcome.txt').read_bytes()
            page.get_by_role('button',name='Mark received',exact=True).click()
            expect(page.locator('main')).to_contain_text('Received.')
            audit('result')
            page.screenshot(path=str(screenshots/'result-mobile.png'),full_page=True)
            page.goto(origin+'/agents')
            page.locator('#devin-connect summary').first.click()
            expect(page.get_by_label('Devin token')).to_be_visible()
            assert page.evaluate('document.documentElement.scrollWidth <= innerWidth')
            audit('agents')
            page.screenshot(path=str(screenshots/'agents-mobile.png'),full_page=True)
            page.set_viewport_size({'width':1100,'height':900})
            page.evaluate("async () => { await document.fonts.ready; await new Promise(requestAnimationFrame); await new Promise(requestAnimationFrame); }")
            page.screenshot(path=str(screenshots/'agents-desktop.png'),full_page=True)
            if args.axe:
                for route in ['settings', 'contacts', '']:
                    page.goto(origin+'/'+route)
                    expect(page.locator('main h1')).to_be_visible()
                    audit(route or 'inbox')
            os.kill(proc.pid,signal.SIGINT)
            proc.wait(timeout=15)
            assert proc.returncode==0, output
            assert not workspace.parent.exists(), 'Temporary preview data was not removed'
            print('PASS: one-command preview, Phone choice, browser pairing, agent choice, one request approval, exact download, no MCP edits, clean exit. Native agent and delivery are fixtures.')
            print('Measured preview: 1 command, 2 terminal selections, 3 sign-in/connect buttons, 3 fields; then 1 approval and 1 download. Build/install and existing agent login are prerequisites.')
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
