#!/usr/bin/env python3
"""Check request logs and ensure a blocked log pipe cannot stall the API."""
import concurrent.futures
import contextlib
import datetime
import http.client
import json
from pathlib import Path
import re
import socket
import subprocess
import sys
import tempfile
import time

binary = str(Path(sys.argv[1] if len(sys.argv) > 1 else 'target/release/chuni-chart-rs').resolve())
secret = 'PRIVATE_LOG_CANARY_8ba4f8df'
salt = 'log-test-salt-' + '7c' * 32


@contextlib.contextmanager
def server(root, blocked_pipe=False):
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    config = root / 'config.json'
    config.write_text(json.dumps({'chart_dir': str(root), 'etag_salt': salt,
                                  'render_workers': 1, 'queue_capacity': 0}), encoding='utf-8')
    log_path = root / 'access.log'
    log = None if blocked_pipe else log_path.open('wb')
    proc = subprocess.Popen([binary, 'serve', '--config', str(config), '--listen', f'127.0.0.1:{port}'],
                            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE if blocked_pipe else log)

    def request(path, headers=None, method='GET'):
        conn = http.client.HTTPConnection('127.0.0.1', port, timeout=20)
        try:
            conn.request(method, path, headers=headers or {})
            response = conn.getresponse()
            return response.status, dict(response.getheaders()), response.read()
        finally:
            conn.close()

    try:
        for _ in range(100):
            try:
                if request('/healthz')[0] == 200:
                    break
            except OSError:
                time.sleep(.05)
        else:
            raise AssertionError('server did not start')
        yield request, log_path, port
    finally:
        proc.terminate()
        proc.communicate(timeout=10)
        if log:
            log.close()


with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    (root / '1086_03.c2s').write_text('BPM_DEF 120\nCREATOR ' + secret + '\nTAP 0 96 0 4\n', encoding='utf-8')
    (root / 'bad.c2s').write_text('BPM_DEF NaN\nTAP 0 96 0 4\n', encoding='utf-8')
    (root / 'slow.c2s').write_text('BPM_DEF 120\n' + '\n'.join(f'TAP 0 {i*4} 0 16' for i in range(11000)), encoding='utf-8')
    with server(root) as (request, log_path, _):
        url = '/api/render/1086_03?div=0&judge=1'
        status, headers, png = request(url)
        assert status == 200
        tag = headers['etag']
        assert request(url, {'If-None-Match': tag})[0] == 304
        assert request(url, method='HEAD')[0] == 200
        assert request('/api/render?name=missing')[0] == 404
        assert request('/api/render?name=bad')[0] == 422
        assert request('/api/render?name=1086_03&div=2')[0] == 400
        assert request('/api/render?name=' + 'x' * 3000)[0] == 414
        assert request(url, {'If-None-Match': 'x' * 9000})[0] == 431
        assert request(url + '&' + secret + '=' + salt)[0] == 400
        assert request('/api/render?name=%1B%5B2J%0Aforged%22')[0] == 404
        with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
            results = list(pool.map(lambda _: request('/judge?name=slow'), range(8)))
        assert any(status == 429 for status, _, _ in results)
        end = '/healthz?name=log-end'
        assert request(end, {'Authorization': 'Bearer ' + secret, 'Cookie': salt})[0] == 200
        # Wait for the asynchronous writer before terminating the process.
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            text = log_path.read_text(encoding='utf-8')
            if f'GET {end} HTTP/1.1' in text:
                break
            time.sleep(.01)
        else:
            raise AssertionError('request log was not written')
        pattern = re.compile(r'^(?P<time>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}[+-]\d{2}:\d{2}): \[render\] "(?P<method>\w+) (?P<target>[^"\r\n]*) HTTP/1\.1" (?P<status>\d{3}) (?P<size>\d+|-) (?P<elapsed>\d+\.\d)ms$')
        lines = [line for line in text.splitlines() if '[render]' in line]
        parsed = [pattern.fullmatch(line) for line in lines]
        assert all(parsed), lines
        rows = [m.groupdict() for m in parsed]
        assert {200, 304, 400, 404, 414, 422, 429, 431} <= {int(row['status']) for row in rows}
        assert any(row['target'] == url and row['method'] == 'GET' and row['status'] == '200' and int(row['size']) == len(png) for row in rows)
        assert any(row['target'] == url and row['status'] == '304' and row['size'] == '0' for row in rows)
        assert any(row['method'] == 'HEAD' for row in rows)
        assert all(datetime.datetime.fromisoformat(row['time']).utcoffset() is not None for row in rows)
        assert secret not in text and salt not in text and tag not in text
        assert '\x1b' not in text and '%0Aforged%22' in text and '[redacted]' in text

    # An unread pipe would otherwise block synchronous logging and HTTP replies.
    # Long paths fill both OS pipe buffers and the bounded log queue quickly.
    with server(root, blocked_pipe=True) as (request, _, port):
        conn = http.client.HTTPConnection('127.0.0.1', port, timeout=5)
        try:
            for _ in range(640):
                conn.request('GET', '/' + 'x' * 1900)
                response = conn.getresponse()
                assert response.status == 404
                response.read()
        finally:
            conn.close()
        assert request('/api/render/1086_03')[0] == 200

    print(json.dumps({'request_log_statuses_and_sizes': 'pass', 'local_time_offset': 'pass',
                      'log_secret_redaction_and_control_escaping': 'pass',
                      'blocked_log_pipe_does_not_block_rendering': 'pass'}))
