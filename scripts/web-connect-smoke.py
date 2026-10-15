"""Signed-in web → copied command → real background service → one approval.
Local dev delivery and a deterministic agent reply; native Codex MCP configuration.
The installed-binary route is exercised. Public release download is a separate check.
"""
import argparse, hashlib, json, os, re, shutil, socket, subprocess, tempfile, time, select, tomllib
from pathlib import Path
from playwright.sync_api import sync_playwright, expect

parser=argparse.ArgumentParser()
parser.add_argument('--binary',type=Path,default=Path('target/debug/litecom'))
args=parser.parse_args(); binary=args.binary.resolve()
native_codex=shutil.which('codex'); assert native_codex, 'Install Codex for its real MCP configuration CLI'
shots=Path('.preview/web-connect'); shots.mkdir(parents=True,exist_ok=True)
with tempfile.TemporaryDirectory(prefix='litecom-web-connect-') as folder:
    root=Path(folder); work=root/'work'; work.mkdir(); programs=root/'bin'; programs.mkdir()
    (programs/'litecom').symlink_to(binary)
    (root/'codex').mkdir()
    (work/'notes.txt').write_text('Exact bytes from the requested agent.\n')
    fixture=programs/'codex'
    fixture.write_text('#!/usr/bin/python3\n'+f'''import os,sys,json
from pathlib import Path
if sys.argv[1:2]==['mcp']: os.execv({native_codex!r},[{native_codex!r},*sys.argv[1:]])
sys.stdin.read()
Path('ran').write_text('yes')
out=sys.argv[sys.argv.index('--output-last-message')+1]
Path(out).write_text(json.dumps({{'text':'Here are your notes.','files':[str(Path('notes.txt').resolve())]}}))
'''); fixture.chmod(0o700)
    env=dict(os.environ,LITECOM_HOME=str(root/'config'),CODEX_HOME=str(root/'codex'),PATH=str(programs)+os.pathsep+os.environ['PATH'])
    with socket.socket() as sock: sock.bind(('127.0.0.1',0)); port=sock.getsockname()[1]
    origin=f'http://127.0.0.1:{port}'; log=root/'relay.log'; connection=None
    with log.open('w') as output, sync_playwright() as p:
        relay=subprocess.Popen([binary,'relay','--listen',f'127.0.0.1:{port}','--public-url',origin,'--data',root/'relay','--dev'],env=env,stdout=output,stderr=output)
        browser=p.chromium.launch(channel='chrome',headless=True)
        try:
            page=browser.new_page(viewport={'width':390,'height':844},accept_downloads=True)
            for _ in range(100):
                try:
                    if page.request.get(origin+'/health').ok: break
                except Exception: pass
                time.sleep(.1)
            page.goto(origin+'/workspace')
            page.get_by_label('Your name').fill('Web connection test')
            page.get_by_label('Phone number').fill('+12025550901')
            page.get_by_role('button',name='Send sign-in code').click()
            expect(page.get_by_label('Sign-in code')).to_be_visible()
            code=re.findall(r'Your Litecom sign-in code is (\d{6})',log.read_text())[-1]
            page.get_by_label('Sign-in code').fill(code); page.get_by_role('button',name='Continue',exact=True).click()
            expect(page.get_by_role('heading',name='Your workspace')).to_be_visible()
            page.get_by_role('button',name='Connect local agent',exact=True).click()
            command=page.get_by_label('Terminal command').input_value()
            connection=re.search(r"--connection '([^']+)'",command).group(1)
            page.reload(); expect(page.get_by_label('Terminal command')).to_have_value(command)
            page.screenshot(path=str(shots/'command-mobile.png'),full_page=True)
            result=subprocess.run(command,shell=True,executable='/bin/sh',env=env,cwd=work,stdin=subprocess.DEVNULL,capture_output=True,text=True,timeout=60)
            assert result.returncode==0,(result.stdout,result.stderr)
            assert 'Listening for requests' in result.stdout
            expect(page.locator('#connect-status')).to_contain_text('Connected.',timeout=20000)
            assert not (work/'ran').exists(), 'Agent ran before approval'
            page.screenshot(path=str(shots/'connected-mobile.png'),full_page=True)
            repeated=subprocess.run(command,shell=True,executable='/bin/sh',env=env,cwd=work,stdin=subprocess.DEVNULL,capture_output=True,text=True,timeout=60)
            assert repeated.returncode==0,(repeated.stdout,repeated.stderr)
            # A second person joins without installing an agent, then accepts the contact.
            other_context=browser.new_context(); other=other_context.new_page(); other.goto(origin+'/workspace')
            other.get_by_label('Your name').fill('Ishaan'); other.get_by_label('Phone number').fill('+12025550902')
            other.get_by_role('button',name='Send sign-in code').click()
            expect(other.get_by_label('Sign-in code')).to_be_visible()
            other.get_by_label('Sign-in code').fill(re.findall(r'Your Litecom sign-in code is (\d{6})',log.read_text())[-1])
            other.get_by_role('button',name='Continue',exact=True).click()
            expect(other.get_by_role('heading',name='Your workspace')).to_be_visible()
            page.goto(origin+'/workspace'); page.get_by_role('button',name='Send contact request').click()
            other.reload(); other.get_by_role('button',name='Accept contact').click()
            expect(other.get_by_text('Connected',exact=True)).to_be_visible()
            # Run the exact MCP command registered by native Codex setup.
            config=tomllib.loads((root/'codex/config.toml').read_text())
            entry=next(iter(config['mcp_servers'].values()))
            mcp=subprocess.Popen([entry['command'],*entry['args']],env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
            def rpc(number,method,params):
                mcp.stdin.write(json.dumps({'jsonrpc':'2.0','id':number,'method':method,'params':params})+'\n'); mcp.stdin.flush()
                deadline=time.monotonic()+20
                while time.monotonic()<deadline:
                    if not select.select([mcp.stdout],[],[],.2)[0]: continue
                    message=json.loads(mcp.stdout.readline())
                    if message.get('id')==number:
                        assert 'error' not in message,message
                        return message['result']
                raise AssertionError('MCP response timed out')
            try:
                rpc(1,'initialize',{'protocolVersion':'2025-03-26','capabilities':{},'clientInfo':{'name':'onboarding-test','version':'1'}})
                mcp.stdin.write(json.dumps({'jsonrpc':'2.0','method':'notifications/initialized'})+'\n'); mcp.stdin.flush()
                tools=rpc(2,'tools/list',{})
                assert any(tool['name']=='litecom_ask' for tool in tools['tools'])
                sent=rpc(3,'tools/call',{'name':'litecom_ask','arguments':{'contact':'Ishaan','request':'Please share the launch notes.','request_id':'ask-ishaan'}})
                assert not sent.get('isError'),sent
                sent=rpc(4,'tools/call',{'name':'litecom_ask','arguments':{'contact':'me','request':'Send me the notes file.','request_id':'ask-my-agent'}})
                assert not sent.get('isError'),sent
                task=sent['structuredContent']['request']
            finally:
                mcp.terminate(); mcp.wait(timeout=10)
            page.goto(origin+'/request/'+task['id'])
            expect(page.get_by_role('button',name='Approve',exact=True)).to_be_visible(timeout=30000)
            assert not (work/'ran').exists(), 'Agent ran before approval'
            page.get_by_role('button',name='Approve',exact=True).click()
            expect(page.get_by_role('link',name='notes.txt',exact=True)).to_be_visible(timeout=30000)
            assert (work/'ran').exists()
            with page.expect_download() as download:
                page.get_by_role('link',name='notes.txt',exact=True).click()
            assert Path(download.value.path()).read_bytes()==(work/'notes.txt').read_bytes()
            print('PASS: copied command with zero prompts; native MCP setup and asks to Ishaan/self; actual background service; refresh and repeat; one approval; exact download.')
        finally:
            if connection: subprocess.run([binary,'--connection',connection,'service','remove'],env=env,capture_output=True,timeout=30)
            browser.close(); relay.terminate(); relay.wait(timeout=15)
