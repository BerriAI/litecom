"""Verify a native cargo-dist archive without a checkout or compiler at runtime.

Requires Python 3.12+. Runs only local development delivery; sends no messages.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.error
import urllib.request
import zipfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('archive', type=Path)
parser.add_argument('--background-service', action='store_true',
                    help='Also verify real OS startup and an approved transfer')
args = parser.parse_args()
archive = args.archive.resolve()
expected_hash = Path(str(archive) + '.sha256').read_text().split()[0]
with archive.open('rb') as stream:
    assert hashlib.file_digest(stream, 'sha256').hexdigest() == expected_hash

with tempfile.TemporaryDirectory(prefix='litecom-package-') as directory:
    root = Path(directory)
    extracted = root / 'extracted'
    if archive.suffix == '.zip':
        with zipfile.ZipFile(archive) as bundle:
            bundle.extractall(extracted)
    else:
        with tarfile.open(archive) as bundle:
            bundle.extractall(extracted, filter='data')
    binaries = list(extracted.rglob('litecom.exe' if os.name == 'nt' else 'litecom'))
    binaries = [path for path in binaries if path.is_file()]
    assert len(binaries) == 1, binaries
    binary = binaries[0]
    assert (binary.parent / 'LICENSE').is_file()
    notices = (binary.parent / 'THIRD-PARTY-NOTICES.html').read_bytes()
    expected_notices = Path(__file__).resolve().parent.parent / 'THIRD-PARTY-NOTICES.html'
    assert notices == expected_notices.read_bytes(), 'Packaged third-party notices are stale or incomplete'
    assert b'Third-party notices' in notices
    assert (binary.parent / 'docs' / 'setup.md').is_file()
    isolated = root / 'empty-working-directory'
    isolated.mkdir()
    # No compiler, Node, Python, native agent, or files from the repo are needed.
    env = dict(os.environ, PATH='', LITECOM_HOME=str(root / 'config'))
    version = subprocess.check_output([binary, '--version'], cwd=isolated, env=env, text=True).strip()
    assert version.startswith('litecom '), version
    help_text = subprocess.check_output([binary, '--help'], cwd=isolated, env=env, text=True)
    assert 'guided setup' in help_text
    assert 'relay' in help_text and 'mcp' in help_text

    with socket.socket() as probe:
        probe.bind(('127.0.0.1', 0))
        port = probe.getsockname()[1]
    address = f'127.0.0.1:{port}'
    origin = 'http://' + address
    data = root / 'relay'
    # Ignore developer proxy settings for this loopback-only check.
    http = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with (root / 'relay.log').open('w+') as log:
        process = subprocess.Popen(
            [binary, 'relay', '--listen', address, '--public-url', origin,
             '--data', data, '--dev'], cwd=isolated, env=env, stdout=log, stderr=log,
        )
        try:
            deadline = time.monotonic() + 20
            while True:
                try:
                    with http.open(origin + '/api/capabilities', timeout=2) as response:
                        assert json.load(response)['development'] is True
                    break
                except (urllib.error.URLError, TimeoutError):
                    if process.poll() is not None or time.monotonic() >= deadline:
                        log.seek(0)
                        raise AssertionError('Packaged relay did not start:\n' + log.read())
                    time.sleep(0.1)
            for route, marker in [('/', b'<html'), ('/app.js', b'function'), ('/style.css', b'{')]:
                with http.open(origin + route, timeout=2) as response:
                    assert marker in response.read(), route
            assert (data / 'litecom.db').is_file()
            assert len((data / 'storage.key').read_bytes()) == 32
        finally:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=10)
    print(f'{version}: checksum, standalone CLI, relay, database, and bundled UI passed.')
    if args.background_service:
        subprocess.run([sys.executable, str(Path(__file__).with_name('background-service-smoke.py')),
                        '--binary', str(binary)], check=True)
