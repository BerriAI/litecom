"""Signed-in web → copied command → real background service → one approval.
Local dev delivery and a deterministic agent reply; native Codex MCP configuration.
Use --public-release to exercise anonymous GitHub downloads from a clean installation.
"""
import argparse, hashlib, json, os, re, shutil, socket, subprocess, tempfile, time, select, tomllib
from pathlib import Path
from playwright.sync_api import sync_playwright, expect
from terminal_qa import run_terminal

parser=argparse.ArgumentParser()
parser.add_argument('--binary',type=Path,default=Path('target/debug/litecom'))
install=parser.add_mutually_exclusive_group()
install.add_argument('--fresh-installer',type=Path,help='Mirror directory containing cargo-dist installer and archive; exercises fresh installation without public hosting')
install.add_argument('--public-release',action='store_true',help='Install anonymously from the matching published GitHub release')
args=parser.parse_args(); binary=args.binary.resolve()
fresh_install=bool(args.fresh_installer or args.public_release)
native_codex=shutil.which('codex'); assert native_codex, 'Install Codex for its real MCP configuration CLI'
shots=Path('.preview/web-connect'); shots.mkdir(parents=True,exist_ok=True)
with tempfile.TemporaryDirectory(prefix='litecom-web-connect-') as folder:
    root=Path(folder); work=root/'work'; work.mkdir(); programs=root/'bin'; programs.mkdir()
    if not fresh_install: (programs/'litecom').symlink_to(binary)
    (root/'codex').mkdir()
    (work/'notes.txt').write_text('Exact bytes from the requested agent.\n')
    fixture=programs/'codex'
    protocol = Path(__file__).resolve().parent / 'fixtures/codex_app_server.py'
    fixture.write_text('#!/usr/bin/python3\n'+f'''import os,sys,json,runpy
from pathlib import Path
if sys.argv[1:2]==['mcp']: os.execv({native_codex!r},[{native_codex!r},*sys.argv[1:]])
def reply():
    Path('ran').write_text('yes')
    return {{'text':'Here are your notes.','files':[str(Path('notes.txt').resolve())]}}
runpy.run_path({str(protocol)!r})['serve'](reply)
'''); fixture.chmod(0o700)
    env=dict(os.environ,LITECOM_HOME=str(root/'config'),CODEX_HOME=str(root/'codex'),XDG_CONFIG_HOME=str(root/'xdg'),PATH=str(programs)+os.pathsep+os.environ['PATH'])
    # Always exercise the multi-agent picker, independent of the test machine.
    (programs/'claude').symlink_to(fixture)
    if fresh_install:
        # Preserve the user's shell profile and installer receipt. Only the
        # download boundary is mirrored; cargo-dist installs the real archive.
        env.update(LITECOM_INSTALL_DIR=str(root/'installed'),LITECOM_NO_MODIFY_PATH='1',XDG_CONFIG_HOME=str(root/'xdg'))
    if args.fresh_installer:
        real_curl=shutil.which('curl'); assert real_curl
        mirror=args.fresh_installer.resolve()
        curl=programs/'curl'
        curl.write_text('#!/usr/bin/python3\n'+f"""import os,sys,shutil
from pathlib import Path
for arg in sys.argv[1:]:
    if arg.startswith('https://github.com/BerriAI/litecom/releases/download/'):
        source=Path({str(mirror)!r})/arg.rsplit('/',1)[-1]
        if not source.is_file(): sys.exit(22)
        if '-o' in sys.argv: shutil.copyfile(source,sys.argv[sys.argv.index('-o')+1])
        else: sys.stdout.buffer.write(source.read_bytes())
        sys.exit(0)
os.execv({real_curl!r},[{real_curl!r},*sys.argv[1:]])
"""); curl.chmod(0o700)
    with socket.socket() as sock: sock.bind(('127.0.0.1',0)); port=sock.getsockname()[1]
    origin=f'http://127.0.0.1:{port}'; log=root/'relay.log'; connections=[]
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
            connections.append(connection)
            page.reload(); expect(page.get_by_label('Terminal command')).to_have_value(command)
            page.screenshot(path=str(shots/'command-mobile.png'),full_page=True)
            result=run_terminal(command,env,work,answers=[('Which agent should handle your requests?',b'\r')],timeout=180)
            assert result.returncode==0,(result.stdout,result.stderr)
            assert 'Listening for requests' in result.stdout
            if fresh_install: assert (root/'installed/litecom').is_file()
            expect(page.locator('#connect-status')).to_contain_text('Connected.',timeout=20000)
            assert not (work/'ran').exists(), 'Agent ran before approval'
            first_agent=next(a for a in page.request.get(origin+'/api/agents').json() if a['is_default'])
            assert first_agent['connected'] and first_agent['local']['harness']=='Codex'
            assert first_agent['name']==first_agent['local']['device_name']
            expect(page.locator('#your-agents')).to_contain_text(first_agent['name']+' · Codex')
            page.get_by_role('link',name='Ask '+first_agent['name']+' · Codex',exact=True).click()
            expect(page.get_by_role('combobox',name='Agent',exact=True)).to_have_value(first_agent['id'])
            page.goto(origin+'/workspace')
            page.screenshot(path=str(shots/'connected-mobile.png'),full_page=True)
            repeated=subprocess.run(command,shell=True,executable='/bin/sh',env=env,cwd=work,stdin=subprocess.DEVNULL,capture_output=True,text=True,timeout=60)
            assert repeated.returncode==0,(repeated.stdout,repeated.stderr)
            # A second person joins without installing an agent, and can immediately request workspace agents.
            other_context=browser.new_context(); other=other_context.new_page(); other.goto(origin+'/workspace')
            other.get_by_label('Your name').fill('Ishaan'); other.get_by_label('Phone number').fill('+12025550902')
            other.get_by_role('button',name='Send sign-in code').click()
            expect(other.get_by_label('Sign-in code')).to_be_visible()
            other.get_by_label('Sign-in code').fill(re.findall(r'Your Litecom sign-in code is (\d{6})',log.read_text())[-1])
            other.get_by_role('button',name='Continue',exact=True).click()
            expect(other.get_by_role('heading',name='Your workspace')).to_be_visible()
            other.reload()
            expect(other.get_by_role('link',name='Ask Web connection test’s agent',exact=True)).to_be_visible()
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
                sent=rpc(4,'tools/call',{'name':'litecom_ask','arguments':{'contact':'me','request':'Send me the notes file.','request_id':'ask-my-agent','requires_action':True}})
                assert not sent.get('isError'),sent
                task=sent['structuredContent']['request']
            finally:
                mcp.terminate(); mcp.wait(timeout=10)
            page.goto(origin+'/request/'+task['id'])
            expect(page.locator('button[data-action="approve"]')).to_be_visible(timeout=30000)
            assert not (work/'ran').exists(), 'Agent ran before approval'
            page.locator('button[data-action="approve"]').click()
            expect(page.get_by_role('link',name='notes.txt',exact=True)).to_be_visible(timeout=30000)
            assert (work/'ran').exists()
            with page.expect_download() as download:
                page.get_by_role('link',name='notes.txt',exact=True).click()
            assert Path(download.value.path()).read_bytes()==(work/'notes.txt').read_bytes()
            # A fresh agent session can read and download the same result through MCP.
            mcp=subprocess.Popen([entry['command'],*entry['args']],env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
            try:
                rpc(101,'initialize',{'protocolVersion':'2025-03-26','capabilities':{},'clientInfo':{'name':'result-test','version':'1'}})
                mcp.stdin.write(json.dumps({'jsonrpc':'2.0','method':'notifications/initialized'})+'\n'); mcp.stdin.flush()
                status=rpc(102,'tools/call',{'name':'litecom_status','arguments':{'task_id':task['id']}})
                assert not status.get('isError'),status
                response=status['structuredContent']['response']
                assert response['text']=='Here are your notes.'
                attachment=response['attachments'][0]
                downloaded=rpc(103,'tools/call',{'name':'litecom_download','arguments':{'task_id':task['id'],'artifact_id':attachment['id']}})
                assert not downloaded.get('isError'),downloaded
                assert downloaded['structuredContent']['verified']
                assert Path(downloaded['structuredContent']['path']).read_bytes()==(work/'notes.txt').read_bytes()
                received=rpc(104,'tools/call',{'name':'litecom_receive','arguments':{'task_id':task['id']}})
                assert not received.get('isError'),received
                print('PASS: a new MCP session reads the reply, downloads verified file bytes, and acknowledges receipt.')
            finally:
                mcp.terminate(); mcp.wait(timeout=10)
            # A named second agent must receive its own connection command,
            # not silently connect another worker to the first agent.
            first_config=(root/'config/connections'/f'{connection}.json').read_bytes()
            page.goto(origin+'/agents')
            page.get_by_text('Add another local agent',exact=True).click()
            page.get_by_label('Name',exact=True).fill('Second computer')
            page.get_by_role('button',name='Create agent',exact=True).click()
            expect(page.get_by_role('heading',name='Connect Second computer',exact=True)).to_be_visible()
            second_command=page.get_by_label('Terminal command').input_value()
            second_connection=re.search(r"--connection '([^']+)'",second_command).group(1)
            connections.append(second_connection)
            second_agent=next(a for a in page.request.get(origin+'/api/agents').json() if a['name']=='Second computer')
            result=run_terminal(second_command,env,work,answers=[('Which agent should handle your requests?',b'\r')],timeout=180)
            assert result.returncode==0,(result.stdout,result.stderr)
            expect(page.locator('#connect-status')).to_contain_text('Connected.',timeout=20000)
            second_config=json.loads((root/'config/connections'/f'{second_connection}.json').read_text())
            assert second_config['agent_id']==second_agent['id']
            assert (root/'config/connections'/f'{connection}.json').read_bytes()==first_config
            page.locator('summary[aria-label="Manage Second computer"]').click()
            with page.expect_request(lambda r: r.url==origin+'/api/connections/local' and r.method=='POST') as reconnect:
                page.get_by_role('button',name='Connect Second computer',exact=True).click()
            assert reconnect.value.post_data_json=={'agent_id':second_agent['id']}
            for width in [320,1100]:
                page.set_viewport_size({'width':width,'height':900})
                assert page.evaluate('document.documentElement.scrollWidth <= innerWidth')
                page.screenshot(path=str(shots/f'named-agent-{width}.png'),full_page=True)
            print('PASS: named agent creation connects that exact agent; reconnect targets it; first connection preserved; 320/1100px.')
            # Uninstall one of two real connections through the terminal picker.
            page.goto(origin+'/settings')
            page.get_by_text('Uninstall from this computer',exact=True).click()
            uninstall_command=page.get_by_label('Uninstall command').input_value()
            assert uninstall_command==f"curl -fsSL '{origin}/uninstall.sh' | sh"
            page.screenshot(path=str(shots/'uninstall-mobile.png'),full_page=True)
            ordered=sorted(connections)
            first, remaining=ordered
            kept=(root/'config/connections'/f'{remaining}.json').read_bytes()
            uninstall=uninstall_command if args.public_release else str(binary)+' uninstall'
            cancelled=run_terminal(uninstall,env,work,answers=[('Which connections do you want to uninstall?',b'\r')],timeout=180)
            assert cancelled.returncode==0,cancelled.stdout
            assert 'Nothing selected' in cancelled.stdout
            removed=run_terminal(uninstall,env,work,answers=[('Which connections do you want to uninstall?',b' \r')],timeout=180)
            assert removed.returncode==0,removed.stdout
            assert str(work) in removed.stdout and 'Codex' in removed.stdout
            assert not (root/'config/connections'/f'{first}.json').exists()
            assert (root/'config/connections'/f'{remaining}.json').read_bytes()==kept
            remaining_config=tomllib.loads((root/'codex/config.toml').read_text())
            assert len(remaining_config['mcp_servers'])==1
            assert (root/'config/bin/litecom').exists()
            assert len(page.request.get(origin+'/api/connections').json())==1
            # Remove the last connection; downloads, unrelated MCP config and work survive.
            with (root/'codex/config.toml').open('a') as config_file:
                config_file.write('\n[mcp_servers.keep_me]\ncommand = "unrelated"\n')
            removed=run_terminal(uninstall,env,work,answers=[('Which connections do you want to uninstall?',b' \r')],timeout=180)
            assert removed.returncode==0,removed.stdout
            assert 'Litecom uninstalled' in removed.stdout
            assert not (root/'config/bin/litecom').exists()
            assert list(tomllib.loads((root/'codex/config.toml').read_text())['mcp_servers'])==['keep_me']
            assert not page.request.get(origin+'/api/connections').json()
            assert (work/'notes.txt').exists()
            assert Path(downloaded['structuredContent']['path']).is_file()
            if fresh_install:
                assert not (root/'installed/litecom').exists()
                assert not (root/'xdg/litecom/litecom-receipt.json').exists()
            print('PASS: uninstall picker cancels safely, removes only selected connections, revokes credentials, preserves other MCP entries/files, and removes binaries after the last connection.')
            source='Anonymous public release install. ' if args.public_release else 'Fresh cargo-dist install (mirrored downloads). ' if args.fresh_installer else ''
            print(source+'PASS: copied command with agent selection; native MCP setup and asks to Ishaan/self; actual background service; refresh and repeat; one approval; exact download.')
        finally:
            for connection in connections:
                subprocess.run([binary,'--connection',connection,'service','remove'],env=env,capture_output=True,timeout=30)
            browser.close(); relay.terminate(); relay.wait(timeout=15)
