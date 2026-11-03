#!/usr/bin/env python3
"""Real HTTP queue checks; Linux additionally freezes owned render workers."""
import concurrent.futures
import contextlib
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

binary = str(Path(sys.argv[1] if len(sys.argv) > 1 else 'target/release/chuni-chart-rs').resolve())


@contextlib.contextmanager
def server(root, workers, capacity, wait=10):
    config = root / 'config.json'
    config.write_text(json.dumps({'chart_dir': str(root), 'render_workers': workers,
                                  'queue_capacity': capacity, 'queue_timeout_seconds': wait,
                                  'timeout_seconds': 10, 'etag_salt': 'queue-test-salt-' + '1f' * 32}), encoding='utf-8')
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    proc = subprocess.Popen([binary, 'serve', '--config', str(config), '--listen', f'127.0.0.1:{port}'],
                            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)

    def request(path, headers=None):
        req = urllib.request.Request(f'http://127.0.0.1:{port}{path}', headers=headers or {})
        try:
            response = urllib.request.urlopen(req, timeout=25)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            return response.status, response.headers, response.read()

    try:
        for _ in range(100):
            try:
                if request('/healthz')[0] == 200:
                    break
            except OSError:
                time.sleep(.05)
        else:
            raise AssertionError('server did not start')
        yield proc, request
    finally:
        proc.terminate()
        proc.communicate(timeout=15)


def children(proc):
    result = set()
    for task in Path(f'/proc/{proc.pid}/task').iterdir():
        try:
            result.update(int(pid) for pid in (task / 'children').read_text().split())
        except FileNotFoundError:
            pass
    return result


def freeze_workers(proc, count):
    stopped = set()
    deadline = time.monotonic() + 3
    while len(stopped) < count and time.monotonic() < deadline:
        for pid in children(proc) - stopped:
            try:
                os.kill(pid, signal.SIGSTOP)
                stopped.add(pid)
            except ProcessLookupError:
                pass
        if len(stopped) < count:
            time.sleep(.002)
    assert len(stopped) == count, ('parallel worker count', stopped, count)
    return stopped


def resume_workers(stopped):
    for pid in stopped:
        try:
            os.kill(pid, signal.SIGCONT)
        except ProcessLookupError:
            pass


with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    source = 'BPM_DEF 120\nTAP 0 96 0 4\nTAP 1 0 8 4\n'
    chart = root / 'demo.c2s'
    chart.write_text(source, encoding='utf-8')
    (root / 'slow.c2s').write_text('BPM_DEF 120\n' + '\n'.join(f'TAP 0 {i*4} 0 16' for i in range(11000)), encoding='utf-8')

    # All platforms: more simultaneous requests than workers must wait and succeed.
    with server(root, workers=4, capacity=16) as (_, request):
        with concurrent.futures.ThreadPoolExecutor(max_workers=12) as pool:
            results = list(pool.map(lambda _: request('/judge?name=slow'), range(12)))
        assert all(s == 200 and b.startswith(b'\x89PNG') for s, _, b in results), [s for s, _, _ in results]

    if sys.platform == 'linux':
        # Two frozen workers prove actual parallel rendering. Two further requests
        # wait, overflow is rejected, and matching validators still bypass the queue.
        with server(root, workers=2, capacity=2, wait=2) as (proc, request):
            tag = request('/api/render/demo')[1]['ETag']
            with concurrent.futures.ThreadPoolExecutor(max_workers=10) as pool:
                active = [pool.submit(request, '/judge?name=slow') for _ in range(2)]
                stopped = freeze_workers(proc, 2)
                try:
                    queued = [pool.submit(request, '/api/render/demo') for _ in range(8)]
                    deadline = time.monotonic() + 1
                    while sum(f.done() for f in queued) < 6 and time.monotonic() < deadline:
                        time.sleep(.005)
                    assert sum(f.done() for f in queued) == 6, 'Queue did not hold exactly two requests'
                    for future in queued:
                        if future.done():
                            status, headers, body = future.result()
                            assert status == 429 and b'queue full' in body
                            assert headers['Retry-After'] == '2' and headers['Cache-Control'] == 'no-store'
                    assert request('/api/render/demo', {'If-None-Match': tag})[0] == 304
                    assert request('/healthz')[0] == 200
                    all_queued = [f.result() for f in queued]
                    assert sum(b'wait timed out' in body for _, _, body in all_queued) == 2
                    assert all(status == 429 for status, _, _ in all_queued)
                finally:
                    resume_workers(stopped)
                assert all(f.result()[0] == 200 for f in active)
            assert request('/api/render/demo')[0] == 200
            assert not children(proc), 'Completed workers were not reaped'

        # Source changes during the wait must affect both the image and its ETag.
        with server(root, workers=1, capacity=2) as (proc, request):
            old_status, old_headers, old_png = request('/api/render/demo')
            assert old_status == 200
            with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
                active = pool.submit(request, '/judge?name=slow')
                stopped = freeze_workers(proc, 1)
                try:
                    queued = pool.submit(request, '/api/render/demo', {'If-None-Match': '"miss"'})
                    time.sleep(.15)
                    assert not queued.done(), 'Busy request did not wait'
                    chart.write_text(source.replace('0 96 0 4', '0 96 4 4'), encoding='utf-8')
                finally:
                    resume_workers(stopped)
                assert active.result()[0] == 200
                status, headers, png = queued.result()
                assert status == 200 and png != old_png and headers['ETag'] != old_headers['ETag']
                assert request('/api/render/demo', {'If-None-Match': headers['ETag']})[0] == 304
            assert not children(proc)

    print(json.dumps({'parallel_queue_success': 'pass',
                      'parallel_workers_queue_full_timeout_etag_and_recovery': 'pass' if sys.platform == 'linux' else 'skipped (requires Linux /proc)',
                      'source_changes_while_queued': 'pass' if sys.platform == 'linux' else 'skipped (requires Linux /proc)'}))
