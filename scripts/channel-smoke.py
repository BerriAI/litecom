"""Real CLI/MCP exchange through isolated local accounts; no agent/model calls."""
import argparse
import http.cookiejar
import json
from pathlib import Path
import re
import socket
import subprocess
import tempfile
import time
import urllib.request

parser = argparse.ArgumentParser()
parser.add_argument('--binary', type=Path, default=Path('target/debug/litecom'))
parser.add_argument('--keep', action='store_true', help='Keep the local fixture running for manual browser QA')
args = parser.parse_args()
binary = args.binary.resolve()
root = Path(tempfile.mkdtemp(prefix='litecom-channel-'))
with socket.socket() as sock:
    sock.bind(('127.0.0.1', 0))
    port = sock.getsockname()[1]
origin = f'http://127.0.0.1:{port}'
log_path = root/'relay.log'
log = log_path.open('w')
relay_args = [str(binary), 'relay', '--listen', f'127.0.0.1:{port}', '--public-url', origin, '--data', str(root/'relay'), '--dev']
processes = []
relay = subprocess.Popen(relay_args, stdout=log, stderr=log)
processes.append(relay)
try:
    for attempt in range(100):
        try:
            if urllib.request.urlopen(origin+'/health').status == 200: break
        except OSError: time.sleep(.1)
    def api(client, path, data=None):
        opener, jar = client
        headers = {'Origin': origin, 'Content-Type': 'application/json', 'X-CSRF-Token': next((c.value for c in jar if c.name=='litecom_csrf'), '')}
        request = urllib.request.Request(origin+path, data=None if data is None else json.dumps(data).encode(), headers=headers)
        with opener.open(request, timeout=40) as response:
            return None if response.status==204 else json.load(response)
    def login(name, phone):
        jar = http.cookiejar.CookieJar()
        client = (urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar)), jar)
        challenge = api(client, '/auth/start', {'name':name,'phone':phone})
        code = re.findall(re.escape(phone)+r': Your Litecom sign-in code is (\d{6})', log_path.read_text())[-1]
        api(client, '/auth/verify', {'id':challenge['id'],'code':code})
        return client, api(client, '/api/me')
    a, person_a = login('Moe Example', '+14155550401')
    b, person_b = login('Tin Example', '+14155550402')
    def connection(client, person, name):
        agent = api(client, '/api/agents')[0]['id']
        code = api(client, '/device/start', {'label':'Channel smoke'})
        api(client, '/api/device/'+code['user_code'], {'agent_id':agent})
        token = api(client, '/device/poll', {'device_code':code['device_code']})['token']
        home = root/name
        (home/'connections').mkdir(parents=True)
        path = home/'connections/local-test.json'
        path.write_text(json.dumps({'relay':origin,'token':token,'user_id':person['id'],'agent_id':agent}))
        path.chmod(0o600)
        return home
    ha, hb = connection(a,person_a,'moe'), connection(b,person_b,'tin')
    def cli(home, *cmd, ok=True):
        result = subprocess.run([binary,'--home',home,'channel',*cmd], capture_output=True,text=True,timeout=40)
        assert (result.returncode==0)==ok, result.stderr
        return json.loads(result.stdout) if ok else result.stderr
    # The existing named connection is discovered without pairing/default config.
    opened = cli(ha, 'create', 'Tin', '--topic', 'Installer coordination', '--request-id', 'retry-create', '--session-key', 'moe-existing-chat')
    room = opened['channel']['id']
    participant_a = opened['joined']['participant_id']
    retried = cli(ha, 'create', 'Tin', '--topic', 'Installer coordination', '--request-id', 'retry-create', '--session-key', 'moe-existing-chat')
    assert retried['channel']['id']==room and retried['joined']['participant_id']==participant_a
    assert cli(hb, 'list', 'Moe')[0]['id']==room
    joined = cli(hb, 'join', 'Moe', '--session-key', 'tin-existing-chat')
    participant_b = joined['joined']['participant_id']
    for _ in range(2):
        cli(ha, 'sync', room, '--participant', participant_a, '--message', 'I am handling the installer. Please take the docs.', '--message-id', 'one')
    history = cli(hb, 'sync', room, '--participant', participant_b)
    assert len(history['messages'])==1

    # Negotiate MCP and send a reply through the exact tool schema.
    mcp = subprocess.Popen([binary, '--home', hb, '--connection', 'local-test', 'mcp'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log, text=True)
    processes.append(mcp)
    def rpc(number, method, params):
        mcp.stdin.write(json.dumps({'jsonrpc':'2.0','id':number,'method':method,'params':params})+'\n'); mcp.stdin.flush()
        while True:
            response = json.loads(mcp.stdout.readline())
            if response.get('id')==number:
                assert 'error' not in response, response
                return response['result']
    rpc(1,'initialize',{'protocolVersion':'2024-11-05','capabilities':{},'clientInfo':{'name':'existing-chat-fixture','version':'1'}})
    mcp.stdin.write(json.dumps({'jsonrpc':'2.0','method':'notifications/initialized'})+'\n'); mcp.stdin.flush()
    tools = rpc(2,'tools/list',{})['tools']
    assert sum(t['name'].startswith('litecom_channel_') for t in tools)==6
    result = rpc(3,'tools/call',{'name':'litecom_channel_sync','arguments':{'channel_id':room,'participant_id':participant_b,'after':1,'message':"I'll handle the docs.",'message_id':'reply','wait_seconds':0}})
    assert not result.get('isError'), result

    other = cli(ha,'create','Tin','--topic','<img src=x onerror=alert(1)>')
    assert 'Several channels' in cli(hb,'join','Moe',ok=False)
    cli(ha,'close',other['channel']['id'])
    relay.terminate(); relay.wait(timeout=10)
    relay = subprocess.Popen(relay_args,stdout=log,stderr=log); processes.append(relay)
    for attempt in range(100):
        try:
            if urllib.request.urlopen(origin+'/health').status==200: break
        except OSError: time.sleep(.1)
    assert len(cli(hb,'read',room)['messages'])==2
    assert api(a,'/api/tasks')==[]
    print('PASS: real CLI/MCP exchange; stable create/join/send retries, named connection discovery, ambiguous joins, restart persistence; no tasks created.',flush=True)
    if args.keep:
        (root/'fixture.json').write_text(json.dumps({'url':origin,'channel':room,'moe_home':str(ha),'tin_home':str(hb),'tin_participant':participant_b,'html_channel':other['channel']['id']}))
        print(f'Browser QA fixture: {root}/fixture.json',flush=True)
        while True: time.sleep(1)
    cli(ha,'close',room)
    assert cli(hb,'read',room)['channel']['closed']
finally:
    for process in reversed(processes):
        if process.poll() is None:
            process.terminate()
            try: process.wait(timeout=10)
            except subprocess.TimeoutExpired: process.kill()
    log.close()
    if not args.keep:
        import shutil
        shutil.rmtree(root)
