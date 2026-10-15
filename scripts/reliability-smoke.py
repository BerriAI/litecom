"""Browser recovery and interruption checks. Isolated accounts; local delivery only."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import re
import socket
import subprocess
import tempfile
import time
import uuid

from playwright.sync_api import sync_playwright, expect

parser = argparse.ArgumentParser()
parser.add_argument('--binary', type=Path, default=Path('target/debug/litecom'))
parser.add_argument('--case', choices=['all', 'downloads', 'editing', 'decisions'], default='all')
args = parser.parse_args()
binary = args.binary.resolve()
evidence = Path('.preview/reliability')
evidence.mkdir(parents=True, exist_ok=True)

with tempfile.TemporaryDirectory(prefix='litecom-reliability-') as directory:
    root = Path(directory)
    with socket.socket() as probe:
        probe.bind(('127.0.0.1', 0))
        port = probe.getsockname()[1]
    origin = f'http://127.0.0.1:{port}'
    log_path = root/'relay.log'
    with log_path.open('w') as log, sync_playwright() as p:
        relay = subprocess.Popen([binary, 'relay', '--listen', f'127.0.0.1:{port}',
                                  '--public-url', origin, '--data', root/'relay', '--dev'],
                                 stdout=log, stderr=log)
        browser = p.chromium.launch(channel='chrome', headless=True)
        pages = []
        try:
            requester = browser.new_context(viewport={'width': 1100, 'height': 850}, accept_downloads=True)
            owner = browser.new_context(viewport={'width': 390, 'height': 844}, accept_downloads=True)
            a, b = requester.new_page(), owner.new_page()
            pages = [a, b]
            for attempt in range(100):
                try:
                    if a.request.get(origin+'/api/capabilities').json()['development']:
                        break
                except Exception:
                    time.sleep(.1)
            else:
                raise AssertionError('Local development relay did not start')

            def api(page, path, data=None, expected=None):
                headers = {'Origin': origin}
                headers['X-CSRF-Token'] = next((c['value'] for c in page.context.cookies()
                                                if c['name'] == 'litecom_csrf'), '')
                response = page.request.fetch(origin+path, method='GET' if data is None else 'POST',
                                              data=data, headers=headers)
                assert (response.status == expected if expected is not None else response.ok), (path, response.status, response.text())
                return None if response.status == 204 else response.json()

            def login(page, name, phone):
                page.goto(origin)
                page.get_by_label('Your name').fill(name)
                page.get_by_label('Phone number').fill(phone)
                page.get_by_role('button', name='Send sign-in code').click()
                expect(page.get_by_label('Sign-in code')).to_be_visible()
                code = re.findall(re.escape(phone)+r': Your Litecom sign-in code is (\d{6})', log_path.read_text())[-1]
                page.get_by_label('Sign-in code').fill(code)
                page.get_by_role('button', name='Continue', exact=True).click()
                expect(page.get_by_role('heading', name='Your inbox')).to_be_visible()
                return api(page, '/api/me')

            person_a = login(a, 'Requester', '+14155550371')
            person_b = login(b, 'Owner', '+14155550372')
            api(a, '/api/contacts', {'invite': person_b['invite']})
            api(b, '/api/contacts/'+person_a['id']+'/accept', {})

            def request(text):
                return api(a, '/api/tasks', {'owner_id': person_b['id'], 'agent_id': None,
                                             'request': text, 'idempotency_key': str(uuid.uuid4())})

            def draft(task, text, files=None):
                return api(b, '/api/tasks/'+task['id']+'/draft', {'draft': {
                    'text': text, 'attachment_ids': files or [], 'action': None}})

            def decide(task, decision):
                d = task['draft']
                return api(b, '/api/tasks/'+task['id']+'/decision', {
                    'version': d['version'], 'digest': d['digest'], 'decision': decision})

            if args.case in ('all', 'decisions'):
                task = draft(request('Test a late approval'), 'Original private response')
                b.goto(origin+'/request/'+task['id'])
                expect(b.get_by_role('button', name='Approve', exact=True)).to_be_visible()
                # Hold the old view exactly as a second phone/tab can have it.
                b.route('**/api/tasks/'+task['id'], lambda route: route.fulfill(json=task))
                revised = draft(task, 'Changed and still private')
                b.get_by_role('button', name='Approve', exact=True).click()
                expect(b.get_by_role('alert')).to_contain_text('changed')
                state = api(a, '/api/tasks/'+task['id'])
                assert state['response'] is None and state['draft'] is None
                b.unroute('**/api/tasks/'+task['id'])
                b.reload()
                expect(b.locator('main')).to_contain_text('Changed and still private')
                b.get_by_role('button', name='Decline', exact=True).click()
                expect(b.locator('.badge')).to_have_text('Declined')
                state = api(a, '/api/tasks/'+task['id'])
                assert state['status'] == 'declined' and state['response'] is None
                d = revised['draft']
                api(b, '/api/tasks/'+task['id']+'/decision', {
                    'version': d['version'], 'digest': d['digest'], 'decision': 'approve'}, expected=409)
                print('PASS: stale review cannot approve changed content; a late conflicting decision cannot undo decline.', flush=True)

            if args.case in ('all', 'downloads'):
                task = request('Download with interrupted connections')
                payload = bytes(range(256))*1024
                file = api(b, '/api/tasks/'+task['id']+'/files', {
                    'name': 'résumé & notes.bin', 'media_type': 'application/octet-stream',
                    'data': base64.b64encode(payload).decode()})
                task = draft(task, 'Only these exact bytes', [file['id']])
                decide(task, 'approve')
                a.goto(origin+'/request/'+task['id'])
                link = a.get_by_role('link', name=file['name'], exact=True)
                expect(link).to_be_visible()
                download_path = '**/api/files/'+file['id']
                a.route(download_path, lambda route: route.abort('connectionreset'))
                link.click()
                expect(a.get_by_role('alert')).to_be_visible()
                assert api(a, '/api/tasks/'+task['id'])['downloads'] == []
                a.unroute(download_path)
                a.reload()
                a.route(download_path, lambda route: route.fulfill(body=b'corrupted', content_type='application/octet-stream'))
                a.get_by_role('link', name=file['name'], exact=True).click()
                expect(a.get_by_role('alert')).to_contain_text('did not match')
                assert api(a, '/api/tasks/'+task['id'])['downloads'] == []
                a.unroute(download_path)
                a.reload()
                receipt_path = download_path+'/received'
                a.route(receipt_path, lambda route: route.abort('connectionreset'))
                with a.expect_download() as downloaded:
                    a.get_by_role('link', name=file['name'], exact=True).click()
                assert hashlib.sha256(Path(downloaded.value.path()).read_bytes()).digest() == hashlib.sha256(payload).digest()
                expect(a.locator('#notice')).to_contain_text('Download verified; receipt could not be confirmed')
                assert api(a, '/api/tasks/'+task['id'])['downloads'] == []
                a.unroute(receipt_path)
                with a.expect_download() as downloaded:
                    a.get_by_role('link', name=file['name'], exact=True).click()
                assert Path(downloaded.value.path()).read_bytes() == payload
                expect(a.locator('#notice')).to_have_text('Download verified.')
                assert len(api(a, '/api/tasks/'+task['id'])['downloads']) == 1
                api(b, '/api/tasks/'+task['id']+'/cancel', {})
                a.reload()
                expect(a.get_by_role('link', name=file['name'], exact=True)).to_have_count(0)
                print('PASS: failed/corrupt transfers have no receipt; lost receipt preserves a successful download; retry and revocation work.', flush=True)

            if args.case in ('all', 'editing'):
                task = draft(request('Preserve an in-progress edit'), 'Old draft')
                b.goto(origin+'/request/'+task['id'])
                b.get_by_text('Edit response', exact=True).click()
                expect(b.get_by_role('textbox', name='Reply', exact=True)).to_be_visible()
                draft(task, 'A simultaneous change from another tab')
                filled = []
                def delayed_poll(route):
                    response = route.fetch()
                    if not filled:
                        b.get_by_role('textbox', name='Reply', exact=True).fill('My unsaved typing')
                        filled.append(True)
                    route.fulfill(response=response)
                b.route('**/api/tasks/'+task['id'], delayed_poll)
                b.wait_for_timeout(5000)
                assert filled, 'The background poll did not run'
                expect(b.get_by_role('textbox', name='Reply', exact=True)).to_have_value('My unsaved typing')
                b.unroute('**/api/tasks/'+task['id'])
                b.get_by_role('heading', name='Review request').click()
                b.wait_for_timeout(4500)
                expect(b.get_by_role('textbox', name='Reply', exact=True)).to_have_value('My unsaved typing')
                b.screenshot(path=str(evidence/'preserved-edit-mobile.png'), full_page=True)
                print('PASS: a background update arriving during typing preserves the in-progress edit.', flush=True)

            for page in pages:
                assert page.evaluate('document.documentElement.scrollWidth <= innerWidth'), 'Horizontal overflow'
            print('PASS: browser reliability scenarios complete; no provider delivery or model calls.', flush=True)
        except Exception:
            for index, page in enumerate(pages):
                page.screenshot(path=str(evidence/f'failure-{index}.png'), full_page=True)
                (evidence/f'failure-{index}.txt').write_text(page.locator('main').inner_text())
            raise
        finally:
            browser.close()
            relay.terminate()
            try:
                relay.wait(timeout=10)
            except subprocess.TimeoutExpired:
                relay.kill()
                relay.wait(timeout=10)
