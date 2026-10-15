"""Exercise the actual local-development relay in two isolated browser accounts."""
import argparse
import hashlib
from pathlib import Path
import re
import time
from playwright.sync_api import sync_playwright, expect

parser = argparse.ArgumentParser()
parser.add_argument('--url', default='http://127.0.0.1:4141')
parser.add_argument('--dev-log', type=Path, required=True)
parser.add_argument('--screenshots', type=Path, default=Path('.preview/ui'))
args = parser.parse_args()
args.screenshots.mkdir(parents=True, exist_ok=True)

with sync_playwright() as p:
    browser = p.chromium.launch(channel='chrome', headless=True)
    try:
        alice = browser.new_context(viewport={'width': 1100, 'height': 850}, accept_downloads=True, permissions=['clipboard-read','clipboard-write'])
        bob = browser.new_context(viewport={'width': 390, 'height': 844}, accept_downloads=True, permissions=['clipboard-read','clipboard-write'])
        a, b = alice.new_page(), bob.new_page()
        errors = []
        for page in (a, b):
            page.on('pageerror', lambda e: errors.append(str(e)))
        suffix = str(int(time.time()))[-6:]

        def login(page, name, phone):
            page.goto(args.url)
            page.wait_for_load_state('networkidle')
            assert page.request.get(args.url+'/api/capabilities').json()['development'], 'Only run this test against explicit local development delivery.'
            page.get_by_label('Your name').fill(name)
            page.get_by_label('Phone number').fill(phone)
            page.get_by_role('button', name='Send sign-in code').click()
            expect(page.get_by_label('Sign-in code')).to_be_visible()
            codes = re.findall(re.escape(phone)+r': Your Litecom sign-in code is (\d{6})', args.dev_log.read_text())
            assert codes, 'No local delivery code found.'
            page.get_by_label('Sign-in code').fill(codes[-1])
            page.get_by_role('button', name='Continue', exact=True).click()
            expect(page.get_by_role('heading', name='Inbox')).to_be_visible()

        login(a, 'Theo', '+1202'+suffix+'1')
        login(b, 'Moe', '+1202'+suffix+'2')
        a.goto(args.url+'/contacts')
        a.get_by_role('button', name='Copy invite link').click()
        link = a.evaluate('navigator.clipboard.readText()')
        b.goto(link)
        b.get_by_role('button', name='Send contact request').click()
        expect(b.get_by_role('heading', name='Contacts')).to_be_visible()
        a.reload()
        a.get_by_role('button', name='Accept contact').click()
        expect(a.get_by_role('link', name='Ask Moe', exact=True)).to_be_visible()
        a.get_by_text('Manage', exact=True).click()
        expect(a.get_by_role('button', name='Block')).to_be_visible()
        a.goto(args.url)
        a.get_by_text('Ask an agent', exact=True).click()
        a.get_by_label(re.compile('Who')).select_option(label='Moe')
        a.get_by_label('What do you need?').fill('Please send me the launch notes.')
        a.get_by_role('button', name='Send request', exact=True).click()
        a.wait_for_url('**/request/*')
        task_url = a.url
        b.goto(task_url)
        b.get_by_text('Prepare a response yourself', exact=True).click()
        b.get_by_label('Reply', exact=True).fill('Private draft: the launch is on Friday.')
        payload = b'Litecom file transfer smoke test\nAll original bytes must survive.\n'
        b.get_by_label('Attach files').set_input_files({'name': 'launch-notes.txt', 'mimeType': 'text/plain', 'buffer': payload})
        b.get_by_role('button', name='Save for review').click()
        expect(b.get_by_role('button', name='Approve', exact=True)).to_be_visible()
        a.reload()
        expect(a.locator('main')).not_to_contain_text('the launch is on Friday')
        expect(a.get_by_role('link', name='launch-notes.txt')).to_have_count(0)
        b.screenshot(path=str(args.screenshots/'review-mobile.png'), full_page=True)
        b.get_by_role('button', name='Approve', exact=True).click()
        expect(b.get_by_role('button', name='Revoke access')).to_be_visible()
        expect(a.get_by_role('link', name='launch-notes.txt')).to_be_visible(timeout=10000)
        expect(a.locator('main')).to_contain_text('the launch is on Friday')
        a.screenshot(path=str(args.screenshots/'response-desktop.png'), full_page=True)
        with a.expect_download() as download:
            a.get_by_role('link', name='launch-notes.txt').click()
        downloaded = Path(download.value.path()).read_bytes()
        assert hashlib.sha256(downloaded).digest() == hashlib.sha256(payload).digest()
        a.get_by_role('button', name='Mark received', exact=True).click()
        expect(a.locator('main')).to_contain_text('Received.')
        b.get_by_role('button', name='Revoke access', exact=True).click()
        expect(b.locator('main')).to_contain_text('Access has been revoked.')
        a.reload()
        expect(a.get_by_role('link', name='launch-notes.txt')).to_have_count(0)
        for page in (a,b):
            assert page.evaluate('document.documentElement.scrollWidth <= innerWidth'), 'Horizontal overflow'
        assert not errors, errors
        print('PASS: two phone identities, mutual contacts, request, private draft, approval, exact file download, receipt, revocation, mobile/desktop layout; local delivery only.')
    except Exception:
        for label, page in [('requester', a), ('owner', b)]:
            page.screenshot(path=str(args.screenshots/(label+'-failure.png')), full_page=True)
            print(label+': '+page.locator('main').inner_text())
        raise
    finally:
        browser.close()
