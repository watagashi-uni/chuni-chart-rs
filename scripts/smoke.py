#!/usr/bin/env python3
"""Exercise the real subprocess/HTTP boundary; standard library only."""
import concurrent.futures
import json
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

binary = str(Path(sys.argv[1] if len(sys.argv) > 1 else 'target/release/chuni-chart-rs').resolve())
with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    (root / 'demo.c2s').write_text((Path(__file__).resolve().parents[1] / 'tests/fixtures/demo.c2s').read_text())
    (root / 'bad.c2s').write_text('BPM_DEF NaN\nTAP 0 0 0 1')
    (root / 'large.c2s').write_bytes(b' ' * (2 * 1024 * 1024 + 1))
    (root / 'slow.c2s').write_text('BPM_DEF 120\n' + '\n'.join(f'TAP 0 {i*4} 0 16' for i in range(11000)))
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    process = subprocess.Popen([binary, 'serve', '--chart-dir', directory, '--listen', f'127.0.0.1:{port}', '--timeout', '2'], stderr=subprocess.PIPE)
    def request(path):
        try:
            with urllib.request.urlopen(f'http://127.0.0.1:{port}{path}', timeout=10) as response:
                return response.status, response.read()
        except urllib.error.HTTPError as error:
            return error.code, error.read()
    try:
        for _ in range(100):
            try:
                if request('/healthz')[0] == 200:
                    break
            except OSError:
                time.sleep(.05)
        else:
            raise AssertionError('server did not start')
        for path, status in [('/preview?name=missing', 404), ('/preview?name=..%2Fsecret', 404), ('/preview?name=bad', 422), ('/preview?name=large', 400)]:
            assert request(path)[0] == status, path
        for mode in [0, 1]:
            status, body = request(f'/api/render?name=demo&judge={mode}&format=jpg')
            assert status == 200 and body[:2] == b'\xff\xd8', (status, body[:100])
        with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
            jobs = [pool.submit(request, '/judge?name=slow') for _ in range(8)]
            time.sleep(.1)
            assert request('/healthz')[0] == 200
            results = [job.result() for job in jobs]
        assert any(status == 429 for status, _ in results), results
        assert any(status == 422 and b'timed out' in body for status, body in results), results
        assert request('/healthz')[0] == 200
        status, body = request('/preview?name=demo&format=png')
        assert status == 200 and body[:8] == b'\x89PNG\r\n\x1a\n', (status, body[:100])
        if sys.platform == 'linux':
            # Successful and killed workers must both have been reaped.
            children = []
            for task in Path(f'/proc/{process.pid}/task').iterdir():
                children.extend((task / 'children').read_text().split())
            assert not children, children
        print(json.dumps({'http_images': 'pass', 'malformed_input': 'pass', 'oversize_input': 'pass', 'concurrency_429': 'pass', 'timeout_and_recovery': 'pass', 'child_reaping': 'pass'}))
    finally:
        process.terminate()
        process.wait(timeout=10)
