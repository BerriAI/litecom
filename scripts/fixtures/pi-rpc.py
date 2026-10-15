"""Deterministic pi RPC fixture for testing the unmodified, published pi-acp.

This is test code only. It calls no model and does not implement a Litecom agent.
"""
import json
import os
from pathlib import Path
import sys

if sys.argv[1:] == ['--version']:
    print('0.81.0')
    sys.exit(0)
assert sys.argv[1:] == ['--mode', 'rpc', '--no-themes']
assert 'LITECOM_TOKEN' not in os.environ


def send(value):
    print(json.dumps(value), flush=True)


def respond(request, data):
    send({'type': 'response', 'id': request.get('id'),
          'command': request['type'], 'success': True, 'data': data})


model = {'provider': 'fixture', 'id': 'fixture', 'name': 'Fixture',
         'contextWindow': 100000}
for line in sys.stdin:
    request = json.loads(line)
    method = request['type']
    if method == 'get_state':
        respond(request, {'sessionId': 'fixture', 'model': model,
                          'thinkingLevel': 'medium'})
    elif method == 'get_available_models':
        respond(request, {'models': [model]})
    elif method == 'get_available_thinking_levels':
        respond(request, {'levels': ['medium']})
    elif method == 'get_session_stats':
        respond(request, {})
    elif method == 'get_commands':
        respond(request, {'commands': []})
    elif method == 'get_messages':
        respond(request, {'messages': []})
    elif method == 'set_session_name':
        respond(request, {})
    elif method == 'prompt':
        assert 'Create a note for the' in request['message']
        Path('agent.pid').write_text(str(os.getpid()))
        respond(request, {})
        send({'type': 'agent_start'})
        send({'type': 'message_update', 'assistantMessageEvent': {
            'type': 'thinking_delta', 'delta': 'private thought'}})
        send({'type': 'extension_ui_request', 'id': 'write-note',
              'method': 'confirm', 'title': 'Create the requested note',
              'message': f'Write {Path.cwd() / "note.txt"}: Only written after approval.'})
    elif method == 'extension_ui_response':
        approved = request.get('confirmed') is True
        Path('decision.json').write_text(json.dumps({
            'optionId': 'yes' if approved else 'no'}))
        if approved:
            Path('note.txt').write_text('Only written after approval.\n')
            reply = {'text': 'The requested note is attached.',
                     'files': [str(Path.cwd() / 'note.txt')]}
        else:
            reply = {'text': 'The operation was declined. No note was created.', 'files': []}
        send({'type': 'message_update', 'assistantMessageEvent': {
            'type': 'text_delta', 'delta': json.dumps(reply)}})
        send({'type': 'agent_end', 'messages': []})
        send({'type': 'agent_settled', 'aborted': False})
    elif method == 'abort':
        respond(request, {})
        send({'type': 'agent_settled', 'aborted': True})
    else:
        raise AssertionError(f'Unexpected pi RPC command: {method}')
