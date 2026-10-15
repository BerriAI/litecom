"""Google-first browser journeys with mocked identity/provider APIs.
Actual OIDC signature/security and persistence are exercised by Rust tests.
"""
import json
from pathlib import Path
from playwright.sync_api import sync_playwright, expect
web=Path('src/web'); shots=Path('.preview/google-onboarding'); shots.mkdir(parents=True,exist_ok=True)
origin='http://litecom.test'
with sync_playwright() as p:
    browser=p.chromium.launch(channel='chrome',headless=True)
    try:
        for width in [320,390,1100]:
            for selected in ['email','phone','slack']:
                state={'signed':False,'channel':False,'starts':0,'email_confirmations':0}
                context=browser.new_context(viewport={'width':width,'height':900})
                errors=[]
                def route(request):
                    path=request.request.url.removeprefix(origin).split('?')[0]; status=200
                    if path=='/api/capabilities': data={'google':True,'phone':True,'email':True,'slack':True,'direct_approvals':{'phone':True,'email':True,'slack':True}}
                    elif path=='/api/me':
                        status=200 if state['signed'] else 401; data={'id':'me','name':'Moe','invite':'fixture'}
                    elif path=='/api/auth/google':
                        state['signed']=True
                        request.fulfill(status=302,headers={'location':origin+'/workspace'}); return
                    elif path=='/api/account': data={'has_phone':False,'google_email':'moe@example.com'}
                    elif path=='/api/channels': data=[{'kind':selected,'verified':True}] if state['channel'] else []
                    elif path=='/api/channels/email/google': state['channel']=True; state['email_confirmations']+=1; data={}
                    elif path=='/api/channels/phone/start': state['starts']+=1; data={'id':'phone-challenge'}
                    elif path=='/api/channels/phone/verify':
                        assert request.request.post_data_json=={'id':'phone-challenge','code':'123456'}
                        state['channel']=True; data={}
                    elif path=='/api/channels/slack/start':
                        state['starts']+=1; data={'url':origin+'/fixture-slack'}
                    elif path=='/api/channels/slack/finish': state['channel']=True; data={}
                    elif path=='/fixture-slack':
                        request.fulfill(content_type='text/html',body='<h1>Fixture Slack consent</h1><a href="/slack/connected?state=fixture">Allow</a>'); return
                    elif path=='/api/agents': data=[{'id':'own-agent','name':'Default','kind':'local'}]
                    elif path=='/api/people': data=[{'id':'ishaan','name':'Ishaan','status':'available','agents':[{'name':'Devin'}]}]
                    elif path=='/app.js' or path=='/style.css' or path.startswith('/fonts/'):
                        request.fulfill(path=str(web/path.lstrip('/'))); return
                    elif not path.startswith('/api/'):
                        request.fulfill(path=str(web/'index.html'),content_type='text/html'); return
                    else: raise AssertionError('Unexpected route: '+path)
                    request.fulfill(status=status,content_type='application/json',body=json.dumps(data))
                context.route('**/*',route); page=context.new_page(); page.on('pageerror',lambda e:errors.append(str(e)))
                page.goto(origin+'/'); expect(page.get_by_label('Phone number')).to_have_count(0)
                page.get_by_role('link',name='Continue with Google').click()
                expect(page.get_by_role('heading',name='Where should requests go?')).to_be_visible()
                page.screenshot(path=str(shots/f'channels-{width}.png'),full_page=True)
                page.get_by_role('button',name='Connect '+selected.title(),exact=True).click()
                if selected=='phone':
                    page.get_by_label('Phone number').fill('+12025550100'); page.get_by_role('button',name='Send code').click()
                    expect(page.get_by_label('Phone code')).to_be_visible()
                    page.reload(); expect(page.get_by_label('Phone code')).to_be_visible()
                    page.get_by_label('Phone code').fill('123456'); page.get_by_role('button',name='Connect phone',exact=True).click()
                    assert state['starts']==1
                elif selected=='slack':
                    page.get_by_role('link',name='Allow').click(); assert state['starts']==1
                else: assert state['email_confirmations']==1
                expect(page.get_by_role('heading',name='Your workspace')).to_be_visible()
                expect(page.get_by_role('heading',name='Ishaan')).to_be_visible()
                expect(page.get_by_role('button',name='Add contact')).to_be_visible()
                expect(page.get_by_role('button',name='Connect local agent')).to_be_visible()
                page.reload(); expect(page.get_by_role('heading',name='Your workspace')).to_be_visible()
                assert page.evaluate('document.documentElement.scrollWidth <= innerWidth')
                assert not errors,errors
                page.screenshot(path=str(shots/f'workspace-{selected}-{width}.png'),full_page=True)
                context.close()
        print('PASS: Google-first UI → Email / Phone / Slack → workspace; no required agent or phone for email/Slack; refresh/resume; 320/390/1100px. Identity/provider APIs are fixtures.')
    finally: browser.close()
