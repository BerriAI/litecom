"""Verify a published update with an older worker and an active fixture request.

Uses temporary local accounts and an OS startup service. No model calls or
external messages. --binary is the new CLI; --old-binary is a prior release.
The new CLI's version must already be published on GitHub.
"""
import argparse
import hashlib
import http.cookiejar
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
import uuid
import runpy
wait_for = runpy.run_path(str(Path(__file__).with_name('background-service-smoke.py')))['wait_for']

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', type=Path, required=True)
parser.add_argument('--old-binary', type=Path, required=True)
args = parser.parse_args()
new, old = args.binary.resolve(strict=True), args.old_binary.resolve(strict=True)
connection = 'qa-update-' + uuid.uuid4().hex
with tempfile.TemporaryDirectory(prefix='litecom update ') as directory:
    root = Path(directory)
    home, work, programs = root/'home', root/'work', root/'bin'
    work.mkdir(); programs.mkdir()
    cli = programs/('litecom.exe' if os.name=='nt' else 'litecom')
    shutil.copy2(new, cli)
    env = dict(os.environ, LITECOM_HOME=str(home))
    env['PATH'] = str(programs) + os.pathsep + env.get('PATH', '')
    fixture = programs/'agent.py'
    protocol = Path(__file__).resolve().parent/'fixtures/codex_app_server.py'
    fixture.write_text("import runpy, time\nfrom pathlib import Path\n"
        "def reply():\n"
        "    with Path('runs').open('a') as f: f.write('run\\n')\n"
        "    while not Path('continue').exists(): time.sleep(.1)\n"
        "    return {'text':'Finished exactly once.', 'files':[]}\n"
        f"runpy.run_path({str(protocol)!r})['serve'](reply)\n")
    if os.name == 'nt':
        (programs/'codex.cmd').write_text(f'@echo off\r\n"{sys.executable}" "{fixture}" %*\r\n')
    else:
        (programs/'codex').write_text(f'#!/bin/sh\nexec {shlex.quote(sys.executable)} {shlex.quote(str(fixture))} "$@"\n')
        (programs/'codex').chmod(0o700)
    def command(binary, *arguments):
        p = subprocess.run([str(binary), '--connection', connection, *map(str,arguments)], env=env,cwd=work,
                           capture_output=True,text=True,timeout=90)
        assert p.returncode==0, p.stdout+p.stderr
        return p.stdout
    with socket.socket() as sock:
        sock.bind(('127.0.0.1',0)); port=sock.getsockname()[1]
    origin=f'http://127.0.0.1:{port}'
    log=root/'relay.log'
    out=log.open('w')
    relay=subprocess.Popen([new,'relay','--listen',f'127.0.0.1:{port}','--public-url',origin,'--data',root/'relay','--dev'],stdout=out,stderr=out)
    jar=http.cookiejar.CookieJar()
    client=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))
    def api(path,body=None):
        request=urllib.request.Request(origin+path, data=None if body is None else json.dumps(body).encode(),headers={
            'Origin':origin,'Content-Type':'application/json','X-CSRF-Token':next((c.value for c in jar if c.name=='litecom_csrf'),'')})
        with client.open(request,timeout=20) as r: return json.load(r) if r.status!=204 else None
    def healthy():
        try: return api('/health')['status']=='ok'
        except OSError: return False
    installed=False; updater=None
    try:
        wait_for('relay',healthy)
        challenge=api('/auth/start',{'name':'Update fixture','phone':'+12025550877'})
        code=re.findall(r'Your Litecom sign-in code is (\d{6})',log.read_text())[-1]
        person=api('/auth/verify',{'id':challenge['id'],'code':code})['profile']
        agent=api('/api/agents')[0]
        pairing=api('/api/connections/local',{'agent_id':agent['id']})
        installed=True
        command(old,'setup','--relay',origin,'--connect-code',pairing['device_code'],'--agent','codex',
                '--workspace',work,'--no-mcp','--no-browser','--no-permission-check')
        saved={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in (home/'connections').iterdir() if p.is_file()}
        definition=None
        if sys.platform=='darwin': definition=Path.home()/f'Library/LaunchAgents/org.litecom.{connection}.plist'
        if sys.platform=='linux': definition=Path.home()/f'.config/systemd/user/org.litecom.{connection}.service'
        before_definition=definition.read_bytes() if definition else None
        task=api('/api/tasks',{'owner_id':person['id'],'agent_id':agent['id'],'request':'Do the fixture work.', 'idempotency_key':'update-fixture'})
        path='/api/tasks/'+task['id']
        task=wait_for('approval',lambda:api(path),lambda t:t['status']=='awaiting_approval')
        api(path+'/decision',{'version':task['draft']['version'],'digest':task['draft']['digest'],'decision':'approve'})
        wait_for('active fixture',lambda:(work/'runs').exists())
        update_log=root/'update.log'
        with update_log.open('w') as stream:
            updater=subprocess.Popen([cli,'update','--wait','120'],env=env,cwd=work,stdout=stream,stderr=stream)
            wait_for('update waiting',lambda:update_log.read_text(),lambda t:'Waiting for active requests' in t,timeout=120)
            assert updater.poll() is None
            assert api(path)['status']=='executing'
            assert command(home/'bin'/cli.name,'--version').strip()==command(old,'--version').strip()
            (work/'continue').touch()
            assert updater.wait(timeout=120)==0,update_log.read_text()
        assert api(path)['status']=='completed'
        assert command(cli,'--version')==command(home/'bin'/cli.name,'--version')==command(new,'--version')
        assert saved=={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in (home/'connections').iterdir() if p.is_file()}
        if definition: assert definition.read_bytes()==before_definition
        assert 'running' in command(cli,'service','status')
        # A new preparation proves that the updated service still has the saved PATH.
        queued=api('/api/tasks',{'owner_id':person['id'],'agent_id':agent['id'],'request':'Prepare another fixture.', 'idempotency_key':'after-update'})
        wait_for('new worker',lambda:api('/api/tasks/'+queued['id']),lambda t:t['status']=='awaiting_approval')
        assert (work/'runs').read_text()=='run\n'
        assert 'up to date' in command(cli,'upgrade')
        print('PASS: verified published download; active request finished once; CLI and worker updated; config and service environment preserved; worker reconnected; repeat update is a no-op.')
    finally:
        (work/'continue').touch()
        if updater and updater.poll() is None: updater.terminate(); updater.wait(timeout=10)
        if installed:
            subprocess.run([old,'--connection',connection,'service','remove'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=45)
        relay.terminate(); relay.wait(timeout=10);out.close()
