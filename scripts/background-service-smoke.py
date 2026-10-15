"""Exercise installed OS startup against a real, isolated development relay.

No provider messages or model calls. The Codex subprocess is a file fixture;
MCP configuration is covered separately by the native-agent setup tests.
"""

import argparse
import hashlib
import http.cookiejar
import json
import os
from pathlib import Path
import re
import shlex
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import uuid


def wait_for(description, read, matches=bool, timeout=40):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        last = read()
        if matches(last):
            return last
        time.sleep(0.25)
    raise AssertionError(f"Timed out waiting for {description}: {last!r}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    connection = "qa-service-" + uuid.uuid4().hex
    with tempfile.TemporaryDirectory(prefix="litecom service ") as directory:
        root = Path(directory)
        work = root / "working folder"
        work.mkdir()
        programs = root / "programs"
        programs.mkdir()
        home = root / "configuration"
        expected = b"One approved request from an installed background worker.\n"
        fixture = programs / "fixture.py"
        fixture.write_text(
            "import json, sys\nfrom pathlib import Path\n"
            "sys.stdin.read()\n"
            "with Path('runs').open('a') as count: count.write('run\\n')\n"
            f"Path('result.bin').write_bytes({expected!r})\n"
            "reply = sys.argv[sys.argv.index('--output-last-message') + 1]\n"
            "Path(reply).write_text(json.dumps({'text':'Done.',"
            "'files':[str(Path('result.bin').resolve())]}))\n",
            encoding="utf-8",
        )
        if os.name == "nt":
            (programs / "codex.cmd").write_text(
                f'@echo off\r\n"{sys.executable}" "{fixture}" %*\r\n',
                encoding="utf-8",
            )
        else:
            command = programs / "codex"
            command.write_text(
                f"#!/bin/sh\nexec {shlex.quote(sys.executable)} "
                f'{shlex.quote(str(fixture))} "$@"\n',
                encoding="utf-8",
            )
            command.chmod(0o700)
        env = dict(os.environ, LITECOM_HOME=str(home))
        env["PATH"] = str(programs) + os.pathsep + env.get("PATH", "")
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        origin = f"http://127.0.0.1:{port}"
        jar = http.cookiejar.CookieJar()
        client = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))

        def api(path, body=None, raw=False):
            headers = {"Origin": origin}
            if body is not None:
                headers["Content-Type"] = "application/json"
                headers["X-CSRF-Token"] = next(
                    (cookie.value for cookie in jar if cookie.name == "litecom_csrf"), ""
                )
            request = urllib.request.Request(
                origin + path,
                data=None if body is None else json.dumps(body).encode(),
                headers=headers,
            )
            with client.open(request, timeout=10) as response:
                data = response.read()
                return data if raw else (json.loads(data) if data else None)

        def cli(*arguments, check=True):
            result = subprocess.run(
                [str(binary), "--connection", connection, *map(str, arguments)],
                env=env, cwd=work, stdin=subprocess.DEVNULL,
                capture_output=True, text=True, timeout=60,
            )
            if check and result.returncode:
                # Never print connection codes or saved credential files.
                raise AssertionError(result.stdout + result.stderr)
            return result

        def running():
            return cli("service", "status").stdout.strip() == "Background worker is running."

        log = root / "relay.log"
        with log.open("w", encoding="utf-8") as output:
            relay = subprocess.Popen(
                [str(binary), "relay", "--listen", f"127.0.0.1:{port}",
                 "--public-url", origin, "--data", str(root / "relay"), "--dev"],
                env=env, stdout=output, stderr=output,
            )
            installed = False
            try:
                def healthy():
                    if relay.poll() is not None:
                        raise AssertionError("Development relay stopped before becoming ready.")
                    try:
                        return api("/health").get("status") == "ok"
                    except urllib.error.URLError:
                        return False

                wait_for("relay", healthy)
                challenge = api("/auth/start", {"name": "Service test", "phone": "+12025550876"})
                code = wait_for(
                    "development sign-in code",
                    lambda: re.findall(r"Your Litecom sign-in code is (\d{6})", log.read_text()),
                )[-1]
                profile = api("/auth/verify", {"id": challenge["id"], "code": code})["profile"]
                agent = next(a for a in api("/api/agents") if a["is_default"])
                pairing = api("/api/connections/local", {"agent_id": agent["id"]})
                setup = ["setup", "--relay", origin, "--connect-code", pairing["device_code"],
                         "--agent", "codex", "--workspace", str(work), "--no-mcp", "--no-browser"]
                installed = True  # Clean up even when installation partially succeeds.
                result = cli(*setup)
                assert "Listening for requests" in result.stdout
                assert running()
                assert api("/api/connections/local/" + pairing["user_code"])["status"] == "connected"
                assert not (work / "runs").exists(), "Agent ran during setup"
                if os.name == "nt":
                    settings = json.loads(subprocess.check_output([
                        "powershell.exe", "-NoLogo", "-NoProfile", "-NonInteractive", "-Command",
                        f"(Get-ScheduledTask -TaskName 'org.litecom.{connection}').Settings | "
                        "Select-Object ExecutionTimeLimit,DisallowStartIfOnBatteries,"
                        "StopIfGoingOnBatteries,MultipleInstances | ConvertTo-Json -Compress",
                    ], text=True, timeout=15))
                    assert settings["ExecutionTimeLimit"] == "PT0S", settings
                    assert not settings["DisallowStartIfOnBatteries"], settings
                    assert not settings["StopIfGoingOnBatteries"], settings
                    assert settings["MultipleInstances"] == 2, settings  # IgnoreNew
                assert "Listening for requests" in cli(*setup).stdout
                print("PASS: setup, actual worker heartbeat, and repeated setup.", flush=True)

                cli("service", "stop")
                wait_for("stopped service", running, lambda value: not value)
                task = api("/api/tasks", {
                    "owner_id": profile["id"], "agent_id": agent["id"],
                    "request": "Create result.bin and send it to me.",
                    "idempotency_key": "background-service-test",
                })
                task_path = "/api/tasks/" + task["id"]
                time.sleep(1)
                assert api(task_path)["status"] == "queued", "Stopped worker claimed new work"
                cli("service", "start")
                task = wait_for("request approval", lambda: api(task_path),
                                lambda value: value["status"] == "awaiting_approval")
                assert not (work / "runs").exists(), "Agent ran before request approval"
                draft = task["draft"]
                api(task_path + "/decision", {
                    "version": draft["version"], "digest": draft["digest"], "decision": "approve",
                })
                task = wait_for("result delivery", lambda: api(task_path),
                                lambda value: value["status"] in ("completed", "failed"))
                assert task["status"] == "completed", task
                artifact = task["response"]["attachments"][0]
                downloaded = api("/api/files/" + artifact["id"], raw=True)
                assert downloaded == expected
                assert hashlib.sha256(downloaded).hexdigest() == artifact["sha256"]
                assert (work / "runs").read_text() == "run\n"
                print("PASS: stopped queue, restart, approval gate, and exact result transfer.", flush=True)

                cli("service", "stop")
                wait_for("stopped service", running, lambda value: not value)
                cli("service", "start")
                wait_for("restarted service", running)
                time.sleep(3)
                assert api(task_path)["status"] == "completed"
                assert (work / "runs").read_text() == "run\n", "Restart repeated completed work"
                cli("service", "remove")
                installed = False
                wait_for("removed service", running, lambda value: not value)
                print(f"PASS: no duplicate execution and startup removal on {sys.platform}.", flush=True)
            except Exception:
                for path in (home / "runs").glob("**/stderr.log"):
                    print(path.read_text(errors="replace"), file=sys.stderr)
                raise
            finally:
                if installed:
                    cli("service", "remove", check=False)
                relay.terminate()
                try:
                    relay.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    relay.kill()
                    relay.wait()


if __name__ == "__main__":
    main()
