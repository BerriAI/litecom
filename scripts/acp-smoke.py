"""Real terminal/browser/ACP subprocess flow; deterministic agent, local delivery only."""
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
import sys
import tempfile
import time
import urllib.request
from playwright.sync_api import sync_playwright, expect

parser = argparse.ArgumentParser()
parser.add_argument('--binary', type=Path, default=Path('target/debug/litecom'))
parser.add_argument('--agent', choices=['acp','opencode','cursor','pi','openclaw','hermes'], default='acp')
parser.add_argument('--pi-acp', type=Path, help='Test a real installed pi-acp with a fixture pi RPC process')
args = parser.parse_args()
if args.pi_acp and args.agent != 'pi':
    parser.error('--pi-acp requires --agent pi')
binary = args.binary.resolve()
screenshots = Path('.preview/acp')/args.agent
screenshots.mkdir(parents=True, exist_ok=True)

with tempfile.TemporaryDirectory(prefix='litecom-acp-qa-') as directory:
    root = Path(directory)
    work = root/'work'
    work.mkdir()
    fake = root/({'cursor':'cursor-agent','pi':'pi-acp','acp':'acp-fixture'}.get(args.agent,args.agent))
    fake.write_text(f'#!{sys.executable}\n' + '''import json, os, sys
from pathlib import Path
assert 'LITECOM_TOKEN' not in os.environ
assert Path(os.environ['PWD']).resolve() == Path.cwd(), 'PWD must match the approved working folder'
assert sys.argv[1:] == ([] if Path(sys.argv[0]).name in ['acp-fixture','pi-acp'] else ['acp'])
def send(value):
    print(json.dumps(value), flush=True)
def result(id,value):
    send({'jsonrpc':'2.0','id':id,'result':value})
def update(value):
    send({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':'fixture','update':value}})
for line in sys.stdin:
    message=json.loads(line)
    method=message.get('method')
    if method=='initialize':
        assert message['params']['protocolVersion']==1
        result(message['id'],{'protocolVersion':1,'agentCapabilities':{},'authMethods':[]})
    elif method=='session/new':
        assert Path(message['params']['cwd'])==Path.cwd()
        assert message['params']['mcpServers']==[]
        result(message['id'],{'sessionId':'fixture'})
    elif method=='session/prompt':
        Path('agent.pid').write_text(str(os.getpid()))
        prompt_id=message['id']
        update({'sessionUpdate':'agent_thought_chunk','content':{'type':'text','text':'private thought'}})
        send({'jsonrpc':'2.0','id':'permission','method':'session/request_permission','params':{
            'sessionId':'fixture','toolCall':{'toolCallId':'write-note','title':'Create the requested note','kind':'edit','rawInput':{'path':str(Path.cwd()/'note.txt'),'content':'Only written after approval.\\n'}},
            'options':[{'optionId':'always','kind':'allow_always','name':'Always allow'},{'optionId':'once','kind':'allow_once','name':'Allow this write'},{'optionId':'no','kind':'reject_once','name':'Decline'}]}})
    elif message.get('id')=='permission' and 'result' in message:
        outcome=message['result']['outcome']
        Path('decision.json').write_text(json.dumps(outcome))
        if outcome.get('optionId')=='once':
            Path('note.txt').write_text('Only written after approval.\\n')
            reply={'text':'The requested note is attached.','files':[str(Path.cwd()/'note.txt')]}
        else:
            assert outcome.get('optionId')!='always'
            reply={'text':'The operation was declined. No note was created.','files':[]}
        update({'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':json.dumps(reply)}})
        result(prompt_id,{'stopReason':'end_turn'})
    elif method=='session/cancel':
        result(prompt_id,{'stopReason':'cancelled'})
''')
    fake.chmod(0o700)
    if args.agent == 'pi':
        fixture = Path(__file__).parent/'fixtures'/'pi-rpc.py'
        (root/'pi').write_text(f'#!{sys.executable}\n' + fixture.read_text())
        (root/'pi').chmod(0o700)
        if args.pi_acp:
            fake.unlink()
            fake.symlink_to(args.pi_acp.resolve(strict=True))
    env = dict(os.environ, PATH=str(root)+os.pathsep+os.environ['PATH'], LITECOM_HOME=str(root/'config'), LITECOM_TOKEN='do-not-forward-test-value')
    if args.pi_acp:
        # Isolate Pi settings. The fixture omits sessionFile, so the adapter
        # records no session history. The real user's HOME stays unchanged.
        env['PI_CODING_AGENT_DIR'] = str(root/'pi-settings')
        env['PI_ACP_PI_COMMAND'] = str(root/'pi')
        (work/'.pi').mkdir()
        (work/'.pi'/'settings.json').write_text('{"quietStartup":true}')
    with socket.socket() as probe:
        probe.bind(('127.0.0.1',0))
        port = probe.getsockname()[1]
    origin = f'http://127.0.0.1:{port}'
    relay_log = root/'relay.log'
    processes = []
    master = None
    with relay_log.open('w') as log, sync_playwright() as p:
        browser = p.chromium.launch(channel='chrome',headless=True)
        try:
            relay = subprocess.Popen([binary,'relay','--listen',f'127.0.0.1:{port}','--public-url',origin,'--data',root/'relay','--dev'],stdout=log,stderr=log,env=env,start_new_session=True)
            processes.append(relay)
            page = browser.new_page(viewport={'width':390,'height':844},accept_downloads=True)
            for attempt in range(100):
                try:
                    if page.request.get(origin+'/api/capabilities').json()['development']: break
                except Exception:
                    time.sleep(.1)
            else: raise AssertionError('Development relay did not start')
            master,slave = pty.openpty()
            setup = subprocess.Popen([binary,'setup','--relay',origin,*(['--acp',str(fake)] if args.agent=='acp' else ['--agent',args.agent]),'--workspace',work,'--no-background','--no-mcp','--no-browser'],stdin=slave,stdout=slave,stderr=slave,env=env,start_new_session=True)
            processes.append(setup)
            os.close(slave)
            output = ''
            def until(text):
                global output
                deadline = time.monotonic()+30
                while text not in output:
                    assert time.monotonic()<deadline, output
                    if select.select([master],[],[],.1)[0]:
                        output += os.read(master,65536).decode(errors='replace')
                return output
            until('Where do you want to approve requests?')
            os.write(master,b'\r')
            until('?approve=phone')
            page.goto(re.search(re.escape(origin)+r'/connect/[^\s]+\?approve=phone',output).group(0))
            page.get_by_label('Your name').fill('ACP test')
            page.get_by_label('Phone number').fill('+14155550235')
            page.get_by_role('button',name='Send sign-in code').click()
            expect(page.get_by_label('Sign-in code')).to_be_visible()
            code = re.findall(r'Your Litecom sign-in code is (\d{6})',relay_log.read_text())[-1]
            page.get_by_label('Sign-in code').fill(code)
            page.get_by_role('button',name='Continue',exact=True).click()
            page.get_by_role('button',name='Connect agent',exact=True).click()
            until('Start listening:')
            setup.wait(timeout=10)
            assert setup.returncode==0,output
            with (root/'worker.log').open('w') as worker_log:
                worker = subprocess.Popen([binary,'worker'],cwd=work,env=env,stdout=worker_log,stderr=worker_log,start_new_session=True)
                processes.append(worker)
                for mode in ['approve','decline','cancel']:
                    note=work/'note.txt'
                    if note.exists(): note.unlink()
                    text=subprocess.check_output([binary,'ask','me',f'Create a note for the {mode} scenario and send it to me.'],env=env,text=True)
                    url=re.search(re.escape(origin)+r'/request/[a-f0-9-]+',text).group(0)
                    page.goto(url)
                    page.get_by_role('button',name='Approve run',exact=True).click(timeout=15000)
                    expect(page.get_by_role('button',name='Approve once',exact=True)).to_be_visible(timeout=15000)
                    assert not note.exists(), 'Agent acted before tool approval'
                    assert page.evaluate('document.documentElement.scrollWidth <= innerWidth')
                    expect(page.locator('.permission')).to_contain_text(str(work.resolve()/'note.txt'))
                    expect(page.locator('.permission')).to_contain_text('Only written after approval.')
                    assert page.locator('.permission').bounding_box()['y'] < page.get_by_text('Run approved ·', exact=False).bounding_box()['y']
                    if mode=='cancel':
                        pid=int((work/'agent.pid').read_text())
                        page.get_by_role('button',name='Cancel request',exact=True).click()
                        for attempt in range(150):
                            try: os.kill(pid,0)
                            except ProcessLookupError: break
                            time.sleep(.1)
                        else: raise AssertionError('ACP process survived cancellation')
                        assert not note.exists()
                        continue
                    if mode=='approve':
                        page.screenshot(path=str(screenshots/'approve-operation-mobile.png'),full_page=True)
                        page.get_by_role('button',name='Approve once',exact=True).click()
                    else:
                        page.get_by_role('button',name='Decline operation',exact=True).click()
                    expect(page.get_by_role('button',name='Approve',exact=True)).to_be_visible(timeout=15000)
                    decision=json.loads((work/'decision.json').read_text())
                    assert decision['optionId']==(('yes' if args.pi_acp else 'once') if mode=='approve' else 'no')
                    assert note.exists()==(mode=='approve')
                    assert 'private thought' not in page.locator('main').inner_text()
                    page.get_by_role('button',name='Approve',exact=True).click()
                    if mode=='approve':
                        with page.expect_download() as download:
                            page.get_by_role('link',name='note.txt',exact=True).click()
                        assert Path(download.value.path()).read_bytes()==b'Only written after approval.\n'
                    else:
                        expect(page.locator('main')).to_contain_text('No note was created.')
            print(f'PASS: {args.agent} ACP setup, native subprocess, exact one-time approval, declined operation, separate sharing, verified attachment, private thoughts, cancellation.')
        finally:
            browser.close()
            for process in reversed(processes):
                if process.poll() is None:
                    os.killpg(process.pid,signal.SIGTERM)
                    try: process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid,signal.SIGKILL)
                        process.wait(timeout=10)
            if master is not None: os.close(master)
