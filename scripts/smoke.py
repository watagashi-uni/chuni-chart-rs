#!/usr/bin/env python3
"""Exercise the real subprocess/HTTP boundary; standard library only."""
import concurrent.futures
import json
import os
from pathlib import Path
import socket
import signal
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
    (root / 'styles.c2s').write_text((Path(__file__).resolve().parents[1] / 'tests/fixtures/scroll-slide-styles.c2s').read_text())
    (root / 'compat.c2s').write_text('BPM_DEF 120\nTAP 0 0 0 4\nTAP 2 0 0 4\n'
                                  'DCM 0 96 96 0.000001\nDCM 0 192 96 1000\n'
                                  'STP 1 0 96\nCLK 0 0\nFUTURE arbitrary fields\n')
    (root / 'overflow.c2s').write_text('BPM_DEF 120\nTAP 0 96 -2 4\nSLD 0 0 -16 4 384 28 4')
    (root / 'bad.c2s').write_text('BPM_DEF NaN\nTAP 0 0 0 1')
    (root / 'large.c2s').write_bytes(b' ' * (20 * 1024 * 1024 + 1))
    (root / 'slow.c2s').write_text('BPM_DEF 120\n' + '\n'.join(f'TAP 0 {i*4} 0 16' for i in range(11000)))
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    config = root / 'server.json'
    config.write_text(json.dumps({'chart_dir': directory, 'render_workers': 1,
                                  'queue_capacity': 16, 'queue_timeout_seconds': 20,
                                  'timeout_seconds': 5}), encoding='utf-8')
    process = subprocess.Popen([binary, 'serve', '--config', str(config), '--listen', f'127.0.0.1:{port}'], stderr=subprocess.PIPE)
    def request(path, headers=None, include_headers=False):
        try:
            req = urllib.request.Request(f'http://127.0.0.1:{port}{path}', headers=headers or {})
            with urllib.request.urlopen(req, timeout=30) as response:
                result = (response.status, response.read())
                return (*result, response.headers) if include_headers else result
        except urllib.error.HTTPError as error:
            result = (error.code, error.read())
            return (*result, error.headers) if include_headers else result
    try:
        for _ in range(100):
            try:
                if request('/healthz')[0] == 200:
                    break
            except OSError:
                time.sleep(.05)
        else:
            raise AssertionError('server did not start')
        for path, status in [('/preview?name=missing', 404), ('/preview?name=..%2Fsecret', 404), ('/preview?name=bad', 422), ('/preview?name=large', 400), ('/api/render?name=demo&early_attack_only=2', 400)]:
            assert request(path)[0] == status, path
        for mode in [0, 1]:
            status, body = request(f'/api/render?name=demo&judge={mode}&format=jpg')
            assert status == 200 and body[:2] == b'\xff\xd8', (status, body[:100])
        early_png = root / 'early.png'
        subprocess.run([binary, 'render', str(root / 'demo.c2s'), '--early-attack-only', '-o', str(early_png)], check=True, stderr=subprocess.DEVNULL)
        status, body = request('/api/render?name=demo&early_attack_only=1')
        assert status == 200 and body == early_png.read_bytes(), 'HTTP/CLI early mode mismatch'
        status, normal = request('/judge?name=demo')
        assert status == 200 and body != normal, 'Early mode was ignored'
        status, body = request('/preview?name=overflow')
        assert status == 200 and body[:8] == b'\x89PNG\r\n\x1a\n'
        for path, cli_mode in [('/judge?name=overflow', '--judge'), ('/fastattack/overflow', '--early-attack-only')]:
            png = root / 'overflow.png'
            subprocess.run([binary, 'render', str(root / 'overflow.c2s'), cli_mode, '-o', str(png)], check=True, stderr=subprocess.DEVNULL)
            status, body, headers = request(path, include_headers=True)
            assert status == 200 and body == png.read_bytes(), ('off-lane HTTP/CLI mismatch', status)
            assert request(path, {'If-None-Match': headers['ETag']}) == (304, b'')
        inspected = json.loads(subprocess.check_output([binary, 'inspect', str(root / 'overflow.c2s')]))
        assert inspected['judgement_supported']
        assert len(inspected['windows']) == 1 and len(inspected['windows'][0]['lanes']) == 2
        for path, cli_mode in [('/preview?name=styles', []), ('/judge?name=styles', ['--judge']), ('/fastattack/styles', ['--early-attack-only'])]:
            png = root / 'styles.png'
            subprocess.run([binary, 'render', str(root / 'styles.c2s'), *cli_mode, '-o', str(png)], check=True, stderr=subprocess.DEVNULL)
            status, body = request(path)
            assert status == 200 and body == png.read_bytes(), ('SLP/SLIDE style HTTP/CLI mismatch', path, status)
        for path, cli_mode in [('/preview?name=compat', []), ('/judge?name=compat', ['--judge']), ('/fastattack/compat', ['--early-attack-only'])]:
            png = root / 'compat.png'
            subprocess.run([binary, 'render', str(root / 'compat.c2s'), *cli_mode, '-o', str(png)], check=True, stderr=subprocess.DEVNULL)
            status, body, headers = request(path, include_headers=True)
            assert status == 200 and body == png.read_bytes(), ('DCM/unknown-record HTTP/CLI mismatch', path, status)
            assert request(path, {'If-None-Match': headers['ETag']}) == (304, b'')
        status, _, cache_headers = request('/preview?name=demo', include_headers=True)
        assert status == 200 and cache_headers['ETag']
        with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
            jobs = [pool.submit(request, '/judge?name=slow') for _ in range(8)]
            time.sleep(.1)
            assert request('/healthz')[0] == 200
            results = [job.result() for job in jobs]
        summary = [(status, len(body)) for status, body in results]
        assert all(status == 200 for status, _ in results), summary
        # Rendering got faster with bounded pair scans. Do not require a valid
        # chart to be slow: freeze our own worker to exercise wall-time cleanup.
        timeout_checked = False
        if sys.platform == 'linux':
            with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
                job = pool.submit(request, '/judge?name=slow')
                deadline = time.monotonic() + 1.5
                while time.monotonic() < deadline and not job.done():
                    children = set()
                    for task in Path(f'/proc/{process.pid}/task').iterdir():
                        children.update((task / 'children').read_text().split())
                    if children:
                        try:
                            os.kill(int(next(iter(children))), signal.SIGSTOP)
                            timeout_checked = True
                            break
                        except ProcessLookupError:
                            pass
                    time.sleep(.005)
                assert timeout_checked, 'Did not observe the test worker'
                # Matching validators must work while the only render slot is busy.
                cached = request('/preview?name=demo', {'If-None-Match': cache_headers['ETag']})
                assert cached == (304, b''), cached
                # This request waits for the frozen worker to time out, then
                # succeeds without a client retry.
                assert request('/preview?name=demo&zoom=2')[0] == 200
                status, body = job.result()
                assert status == 422 and b'timed out' in body, (status, body[:100])
        assert request('/healthz')[0] == 200
        status, body = request('/preview?name=demo&format=png')
        assert status == 200 and body[:8] == b'\x89PNG\r\n\x1a\n', (status, body[:100])
        if sys.platform == 'linux':
            # Successful and killed workers must both have been reaped.
            children = []
            for task in Path(f'/proc/{process.pid}/task').iterdir():
                children.extend((task / 'children').read_text().split())
            assert not children, children
        print(json.dumps({'http_images': 'pass', 'early_attack_mode': 'pass', 'off_lane_judgements': 'pass', 'scroll_slide_styles': 'pass', 'malformed_input': 'pass', 'oversize_input': 'pass', 'concurrent_requests_wait': 'pass', 'conditional_hit_while_busy': 'pass' if timeout_checked else 'skipped (requires Linux /proc)', 'timeout_and_recovery': 'pass' if timeout_checked else 'skipped (requires Linux /proc)', 'child_reaping': 'pass' if sys.platform == 'linux' else 'skipped'}))
    finally:
        process.terminate()
        process.wait(timeout=10)
