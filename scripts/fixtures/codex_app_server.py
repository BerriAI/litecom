"""Deterministic Codex protocol peer for install/preview smoke tests."""
import json
import sys


def serve(reply):
    assert sys.argv[1:2] == ["app-server"], "Expected a dedicated Codex session"

    def send(message):
        print(json.dumps(message), flush=True)

    for line in sys.stdin:
        message = json.loads(line)
        method = message.get("method")
        if method == "initialize":
            send({"id": message["id"], "result": {}})
        elif method == "thread/start":
            send({"id": message["id"], "result": {"thread": {"id": "fixture-thread"}}})
        elif method == "turn/start":
            send({"id": message["id"], "result": {"turn": {"id": "fixture-turn"}}})
            result = reply()
            send({"method": "item/completed", "params": {
                "threadId": "fixture-thread", "turnId": "fixture-turn",
                "item": {"type": "agentMessage", "text": json.dumps(result)},
            }})
            send({"method": "turn/completed", "params": {
                "threadId": "fixture-thread", "turn": {"id": "fixture-turn", "status": "completed"},
            }})
            return
