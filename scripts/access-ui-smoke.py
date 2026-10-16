"""Approval modes, expired-session recovery and docs layout against an isolated relay.
Uses fixture agents; no model calls or external messages.
"""
import argparse
import json
import markdown
import os
from pathlib import Path
import re
import socket
import sqlite3
import subprocess
import tempfile
import time
import uuid
from playwright.sync_api import sync_playwright, expect

parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary',type=Path,default=Path('target/debug/litecom'))
binary=parser.parse_args().binary.resolve()
evidence=Path('.preview/access-ui');evidence.mkdir(parents=True,exist_ok=True)
with tempfile.TemporaryDirectory(prefix='litecom-access-ui-') as folder, sync_playwright() as pw:
    root=Path(folder)
    with socket.socket() as probe:probe.bind(('127.0.0.1',0));port=probe.getsockname()[1]
    origin=f'http://127.0.0.1:{port}'
    logpath=root/'relay.log'
    with logpath.open('w') as log:
        relay=subprocess.Popen([binary,'relay','--listen',f'127.0.0.1:{port}','--public-url',origin,'--data',root/'relay','--dev'],stdout=log,stderr=log)
        browser=pw.chromium.launch(channel='chrome',headless=True)
        page=browser.new_page(viewport={'width':1100,'height':850})
        errors=[];page.on('pageerror',lambda e: errors.append(str(e)))
        try:
            for _ in range(100):
                try:page.goto(origin);break
                except Exception:time.sleep(.1)
            def login():
                page.get_by_label('Your name',exact=True).fill('Tin')
                page.get_by_label('Phone number',exact=True).fill('+14155550831')
                page.get_by_role('button',name='Send sign-in code',exact=True).click()
                expect(page.get_by_label('Sign-in code',exact=True)).to_be_visible()
                code=re.findall(r'Your Litecom sign-in code is (\d{6})',logpath.read_text())[-1]
                page.get_by_label('Sign-in code',exact=True).fill(code)
                page.get_by_role('button',name='Continue',exact=True).click()
                expect(page.get_by_role('button',name='Continue',exact=True)).not_to_be_visible()
            def call(path,body=None):
                headers={'Origin':origin,'X-CSRF-Token':next((c['value'] for c in page.context.cookies() if c['name']=='litecom_csrf'),'')}
                r=page.request.get(origin+path,headers=headers) if body is None else page.request.post(origin+path,headers=headers,data=body)
                assert r.ok,(path,r.status,r.text())
                return r.json() if r.body() else None
            def fits():
                assert page.evaluate('document.documentElement.scrollWidth <= innerWidth'), 'Horizontal overflow'
                buttons=page.locator('.approval-panel .actions button')
                boxes=[buttons.nth(i).bounding_box() for i in range(buttons.count())]
                for i,a in enumerate(boxes):
                    assert a['height']>=44
                    for b in boxes[i+1:]:
                        assert a['x']+a['width']<=b['x']+.5 or b['x']+b['width']<=a['x']+.5 or a['y']+a['height']<=b['y']+.5 or b['y']+b['height']<=a['y']+.5,'Buttons overlap'
            login();call('/api/channels/phone',{})
            me=call('/api/me');owner=me['id']
            agent=call('/api/agents')[0]['id']
            def draft(task):
                action={'kind':'harness','title':'Run Codex','description':'Run the requested work using your tools.','runner':'codex','executable':'/fixture/codex','instruction':task['request'],'workspace':'/Users/tin','permissions':['Fixture'],'acting_account':'Your Codex account','share_result':True}
                return call('/api/tasks/'+task['id']+'/draft',{'draft':{'text':'','attachment_ids':[],'action':action}})
            page.goto(origin+'/?ask=me')
            page.get_by_label('What do you need?',exact=True).fill('Read the launch notes.')
            expect(page.get_by_role('combobox',name='Access',exact=True)).to_have_value('read_only')
            page.get_by_role('button',name='Send request',exact=True).click();page.wait_for_url('**/request/*')
            task=call('/api/tasks/'+page.url.rsplit('/',1)[1]);assert task['ask_only']
            draft(task);page.reload()
            expect(page.get_by_role('button',name='Approve read-only',exact=True)).to_be_visible()
            expect(page.get_by_role('button',name='Approve actions',exact=True)).not_to_be_visible()
            fits();page.screenshot(path=str(evidence/'read-only-desktop.png'),full_page=True)
            page.set_viewport_size({'width':390,'height':844});fits();page.screenshot(path=str(evidence/'read-only-mobile.png'),full_page=True)
            page.get_by_role('button',name='Approve read-only',exact=True).click()
            expect(page.locator('#app')).to_contain_text('Approved read-only')
            assert call('/api/tasks/'+task['id'])['approved_access']=='read_only'
            action_task=call('/api/tasks',{'owner_id':owner,'agent_id':agent,'request':'Create a launch checklist.','ask_only':False,'idempotency_key':uuid.uuid4().hex})
            draft(action_task);page.goto(origin+'/request/'+action_task['id'])
            expect(page.get_by_role('button',name='Approve actions',exact=True)).to_be_visible()
            expect(page.get_by_role('button',name='Approve read-only',exact=True)).to_be_visible()
            fits();page.screenshot(path=str(evidence/'actions-mobile.png'),full_page=True)
            page.get_by_role('button',name='Decline',exact=True).click()
            expect(page.locator('#app')).to_contain_text('Declined')
            assert call('/api/tasks/'+action_task['id'])['status']=='declined'
            # Keep an unsent request through a browser sign-in. Never auto-submit it.
            page.goto(origin+'/?ask=me')
            page.get_by_label('What do you need?',exact=True).fill('Keep this unsent question')
            call('/auth/logout',{})
            # Age only the isolated fixture's sign-in challenge; retain real rate limits.
            with sqlite3.connect(root/'relay'/'litecom.db') as db:
                db.execute('UPDATE otp SET expires_at=expires_at-3600')
            page.get_by_role('button',name='Send request',exact=True).click()
            expect(page.get_by_role('link',name='Sign in again',exact=True)).to_be_visible()
            expect(page.get_by_label('What do you need?',exact=True)).to_have_value('Keep this unsent question')
            page.get_by_role('link',name='Sign in again',exact=True).click();login()
            expect(page.get_by_label('What do you need?',exact=True)).to_have_value('Keep this unsent question')
            assert len(call('/api/tasks'))==2
            # A draft from another account must not appear after sign-in.
            page.evaluate('''sessionStorage.setItem('resume-ask', JSON.stringify({user:'another-account',path:location.pathname+location.search,at:Date.now(),fields:{request:'Private other-user text'}}))''')
            page.reload()
            expect(page.get_by_label('What do you need?',exact=True)).to_have_value('')
            # The product links to repository docs; render their actual Markdown.
            page.goto('about:blank')
            css = "*{box-sizing:border-box}body{width:100%;font:16px/1.6 system-ui;max-width:900px;margin:24px auto;padding:0 20px;color:#202124}h1{font-size:28px}table{border-collapse:collapse;display:block;max-width:100%;overflow:auto}td,th{padding:8px;border:1px solid #ddd}pre{padding:12px;background:#f5f5f5;overflow:auto}a,code{overflow-wrap:anywhere}code{font-size:13px}img{max-width:100%}"
            for name in ['setup','approval-channels','distribution','local-agent','self-host','agents','hosting-channels','slack-agent']:
                source=Path('docs')/(name+'.md')
                html=markdown.markdown(source.read_text(),extensions=['fenced_code','tables'])
                for width, label in [(390,'mobile'),(1100,'desktop')]:
                    page.set_viewport_size({'width':width,'height':844})
                    page.set_content('<style>'+css+'</style><main>'+html+'</main>')
                    expect(page.locator('h1')).to_have_text(source.read_text().splitlines()[0][2:])
                    page.screenshot(path=str(evidence/(name+'-'+label+'.png')),full_page=True);fits()
            assert not errors,errors
            print('PASS: ask-only default, both approval modes, decline, browser sign-in preserves unsent text without replay, narrow layout, rendered docs.')
        finally:
            browser.close();relay.terminate();relay.wait(timeout=10)
