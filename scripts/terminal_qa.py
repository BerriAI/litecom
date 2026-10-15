"""Run copied shell pipelines with a controlling terminal, as a person does."""
import fcntl
import os
import pty
import select
import signal
import subprocess
import termios
import time


def run_terminal(command, env, cwd, answers=(), timeout=90):
    master, slave = pty.openpty()
    process = subprocess.Popen(
        command, shell=True, executable='/bin/sh', env=env, cwd=cwd,
        stdin=slave, stdout=slave, stderr=slave, start_new_session=True,
        preexec_fn=lambda: fcntl.ioctl(0, termios.TIOCSCTTY, 0),
    )
    os.close(slave)
    output = ''
    pending = list(answers)
    deadline = time.monotonic() + timeout
    try:
        while time.monotonic() < deadline:
            if select.select([master], [], [], .1)[0]:
                try:
                    chunk = os.read(master, 65536)
                except OSError:
                    break
                if not chunk:
                    break
                output += chunk.decode(errors='replace')
                if pending and pending[0][0] in output:
                    _, answer = pending.pop(0)
                    os.write(master, answer)
            elif process.poll() is not None:
                break
        if process.poll() is None:
            process.wait(timeout=2)
        assert not pending, (pending, output)
        return subprocess.CompletedProcess(command, process.returncode, output, '')
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            process.wait(timeout=10)
        os.close(master)
