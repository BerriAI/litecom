"""Browser flow with explicit local API/OAuth fixtures; sends no Slack messages."""
import json
from pathlib import Path
from playwright.sync_api import sync_playwright, expect

web = Path(__file__).resolve().parents[1] / 'src/web'
screenshots = web.parents[1] / '.preview/slack-onboarding'
screenshots.mkdir(parents=True, exist_ok=True)
origin = 'http://litecom.test'

with sync_playwright() as p:
    browser = p.chromium.launch(channel='chrome', headless=True)
    try:
        for width in (320, 390, 1100):
            context = browser.new_context(viewport={'width': width, 'height': 900})
            state = {'signed_in': False, 'connected': False, 'starts': 0, 'fail': False}
            errors = []

            def route(request):
                path = request.request.url.removeprefix(origin).split('?')[0]
                status = 200
                if path == '/api/capabilities':
                    data = {'phone': True, 'slack': True, 'email': True, 'development': True}
                elif path == '/api/me':
                    status = 200 if state['signed_in'] else 401
                    data = {'id': 'owner', 'name': 'Moe'}
                elif path == '/auth/start':
                    data = {'id': 'fixture-code'}
                elif path == '/auth/verify':
                    state['signed_in'] = True
                    data = {}
                elif path == '/api/account':
                    data = {'has_phone': True, 'google_email': None}
                elif path == '/api/channels':
                    data = [{'kind': 'slack', 'verified': True}] if state['connected'] else []
                elif path == '/api/channels/slack/start':
                    state['starts'] += 1
                    status = 503 if state['fail'] else 200
                    data = {'error': {'message': 'Slack could not be reached. Please retry.'}} if state['fail'] else {'url': origin + '/fixture-slack'}
                elif path == '/api/channels/slack/finish':
                    state['connected'] = True
                    data = {}
                elif path == '/api/agents':
                    data = [{'id': 'agent', 'name': 'My agent', 'is_default': True}]
                elif path.startswith('/api/device/'):
                    data = {'label': 'My computer'}
                elif path == '/fixture-slack':
                    request.fulfill(content_type='text/html', body='<h1>Fixture Slack consent</h1>')
                    return
                elif path == '/app.js' or path == '/style.css' or path.startswith('/fonts/'):
                    file = web / path.lstrip('/')
                    request.fulfill(path=str(file))
                    return
                elif not path.startswith('/api/'):
                    request.fulfill(path=str(web / 'index.html'), content_type='text/html')
                    return
                else:
                    raise AssertionError('Unexpected request: ' + path)
                request.fulfill(status=status, content_type='application/json', body=json.dumps(data))

            context.route('**/*', route)
            page = context.new_page()
            page.on('pageerror', lambda error: errors.append(str(error)))
            start = origin + '/connect/FIXTURE?approve=slack'
            page.goto(start)
            page.get_by_label('Your name').fill('Moe')
            page.get_by_label('Phone number').fill('+12025550100')
            page.get_by_role('button', name='Send sign-in code').click()
            page.get_by_label('Sign-in code').fill('123456')
            page.get_by_role('button', name='Continue', exact=True).click()
            page.wait_for_url(origin + '/fixture-slack')
            assert state['starts'] == 1, state

            # Back and refresh must never trap someone in repeated OAuth redirects.
            page.go_back()
            expect(page.get_by_role('button', name='Continue to Slack')).to_be_visible()
            page.reload()
            expect(page.get_by_role('button', name='Continue to Slack')).to_be_visible()
            assert state['starts'] == 1, state
            assert page.evaluate('document.documentElement.scrollWidth <= innerWidth')
            page.screenshot(path=str(screenshots / f'resume-{width}.png'), full_page=True)

            # Failed starts retain a keyboard-accessible retry, including on a fresh URL.
            state['fail'] = True
            page.goto(origin + '/connect/RETRY?approve=slack')
            expect(page.get_by_role('alert')).to_contain_text('Please retry')
            retry = page.get_by_role('button', name='Try again')
            expect(retry).to_be_enabled()
            page.screenshot(path=str(screenshots / f'error-{width}.png'), full_page=True)
            state['fail'] = False
            retry.focus()
            retry.press('Enter')
            page.wait_for_url(origin + '/fixture-slack')

            # A cancelled consent screen requires an explicit retry.
            starts = state['starts']
            page.goto(origin + '/slack/connected?error=cancelled')
            expect(page.get_by_role('heading', name='Slack was not connected.')).to_be_visible()
            expect(page.get_by_role('button', name='Try again')).to_be_enabled()
            assert state['starts'] == starts

            # Completion returns to the original pairing route without another Slack step.
            page.goto(origin + '/slack/connected?state=fixture-state')
            expect(page.get_by_role('heading', name='Connect this agent?')).to_be_visible()
            assert page.url == origin + '/connect/RETRY?approve=slack'
            assert state['starts'] == starts
            page.reload()
            expect(page.get_by_role('heading', name='Connect this agent?')).to_be_visible()
            assert not errors, errors
            context.close()
        print('PASS: automatic Slack handoff, Back/refresh, failure retry, cancellation, and return to pairing at 320/390/1100px. API and OAuth fixtures only.')
    finally:
        browser.close()
