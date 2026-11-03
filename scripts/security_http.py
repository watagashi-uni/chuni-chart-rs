#!/usr/bin/env python3
"""Black-box security and conditional-GET checks against the real server binary."""
import contextlib
import hashlib
import http.client
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request

binary = str(Path(sys.argv[1] if len(sys.argv) > 1 else 'target/release/chuni-chart-rs').resolve())
marker = 'PRIVATE_SOURCE_CANARY_28373f4199104be0'
salt = 'test-only-' + '1f' * 32


@contextlib.contextmanager
def server(config_path):
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    proc = subprocess.Popen([binary, 'serve', '--config', str(config_path), '--listen', f'127.0.0.1:{port}'],
                            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    def request(path, headers=None, method='GET'):
        req = urllib.request.Request(f'http://127.0.0.1:{port}{path}', headers=headers or {}, method=method)
        try:
            response = urllib.request.urlopen(req, timeout=15)
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
        yield request, port
    finally:
        proc.terminate()
        try:
            _, stderr = proc.communicate(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
            _, stderr = proc.communicate(timeout=5)
            raise AssertionError('server did not stop')
        assert marker.encode() not in stderr and salt.encode() not in stderr, 'Source/secret in server log'


def image(response):
    status, headers, body = response
    assert status == 200 and body.startswith(b'\x89PNG\r\n\x1a\n'), (status, body[:100])
    tag = headers.get('ETag', '')
    assert tag.startswith('W/"') and tag.endswith('"') and len(tag) == 68, tag
    assert headers['Cache-Control'] == 'no-cache'
    assert headers['X-Content-Type-Options'] == 'nosniff'
    assert marker.encode() not in body and salt.encode() not in body
    return tag, body


with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    charts = root / 'charts'
    settings = root / 'settings'
    charts.mkdir()
    settings.mkdir()
    source = 'BPM_DEF 120\nCREATOR ' + marker + '\nTAP 0 96 0 4\nCHR 0 115.2 0 4\nTAP 1 0 8 4\n'
    chart = charts / '1086_03.c2s'
    chart.write_text(source, encoding='utf-8')
    (charts / 'invalid.c2s').write_text(marker, encoding='utf-8')
    (charts / 'binary.c2s').write_bytes(b'\xff\x00' + marker.encode())
    (charts / 'large.c2s').write_bytes(b' ' * (2 * 1024 * 1024 + 1))
    (charts / 'directory.c2s').mkdir()
    outside = root / 'outside.c2s'
    outside.write_text(source.replace('0 4', '4 4'), encoding='utf-8')
    symlinks = False
    try:
        (charts / 'escape.c2s').symlink_to(outside)
        (charts / 'alias.c2s').symlink_to(chart)
        symlinks = True
    except OSError:
        if os.name != 'nt':
            raise
    if hasattr(os, 'mkfifo'):
        os.mkfifo(charts / 'pipe.c2s')
    config = settings / 'server.json'
    config.write_text(json.dumps({'chart_dir': '../charts', 'etag_salt': salt, 'timeout_seconds': 2}), encoding='utf-8')
    # Config source and salt must not be exposed, even when copied into chart_dir.
    (charts / 'config.json').write_text(config.read_text(encoding='utf-8'), encoding='utf-8')

    with server(config) as (request, port):
        base = '/api/render?name=1086_03'
        tag, png = image(request(base))
        assert hashlib.sha256(source.encode()).hexdigest() not in tag, 'Unsalted source digest leaked'
        assert image(request('/api/render/1086_03')) == (tag, png)
        assert image(request('/preview?name=1086_03.c2s')) == (tag, png)
        for validator in (tag, tag.removeprefix('W/'), '"unrelated", ' + tag, '*'):
            status, headers, body = request(base, {'If-None-Match': validator})
            assert status == 304 and not body and headers['ETag'] == tag
            assert headers['Cache-Control'] == 'no-cache'
        # Multiple If-None-Match header fields use the same weak comparison.
        connection = http.client.HTTPConnection('127.0.0.1', port, timeout=10)
        connection.putrequest('GET', base)
        connection.putheader('If-None-Match', '"unrelated"')
        connection.putheader('If-None-Match', tag)
        connection.endheaders()
        response = connection.getresponse()
        assert response.status == 304 and not response.read()
        connection.close()
        assert request(base, {'If-None-Match': '"different"'})[0] == 200
        assert request(base, {'If-None-Match': tag}, method='HEAD')[0] == 304
        status, headers, body = request(base, method='HEAD')
        assert status == 200 and not body and headers['ETag'] == tag
        # A timestamp-only change must not invalidate a content validator.
        os.utime(chart, None)
        assert request(base, {'If-None-Match': tag})[0] == 304
        # Changing bytes while retaining size and mtime must invalidate it.
        stat = chart.stat()
        chart.write_text(source.replace('TAP 0 96 0 4', 'TAP 0 96 4 4'), encoding='utf-8')
        os.utime(chart, ns=(stat.st_atime_ns, stat.st_mtime_ns))
        changed_tag, changed_png = image(request(base, {'If-None-Match': tag}))
        assert changed_tag != tag and changed_png != png
        chart.write_text(source, encoding='utf-8')
        assert image(request(base))[0] == tag
        # Identical semantics through aliases share a validator; changed output
        # options never reuse an incompatible cached image.
        judge_tag, _ = image(request('/judge?name=1086_03', {'If-None-Match': tag}))
        fast_tag, fast_png = image(request('/api/render/1086_03?fastattack=1', {'If-None-Match': judge_tag}))
        assert len({tag, judge_tag, fast_tag}) == 3
        for endpoint in ('/fastattack/1086_03', '/fastattack?name=1086_03',
                         base + '&early_attack_only=1', base + '&fastattack=1&judge=0'):
            assert request(endpoint, {'If-None-Match': fast_tag})[0] == 304, endpoint
        cli_png = root / 'fast.png'
        subprocess.run([binary, 'render', str(chart), '--early-attack-only', '-o', str(cli_png)], check=True, stderr=subprocess.DEVNULL)
        assert fast_png == cli_png.read_bytes()
        for suffix in ('&div=0', '&zoom=2', '&column=1', '&format=jpg', '&judge=1&easy=1'):
            status, headers, body = request(base + suffix, {'If-None-Match': tag})
            assert status == 200 and headers['ETag'] != tag, (suffix, status)
        jpeg_tag = request(base + '&format=jpg')[1]['ETag']
        assert request(base + '&format=jpeg', {'If-None-Match': jpeg_tag})[0] == 304
        assert request('/api/render?name=invalid', {'If-None-Match': '*'})[0] == 422
        cases = [
            ('/api/render?name=missing', 404), ('/api/render?name=directory', 404),
            ('/api/render?name=large', 400), ('/api/render?name=binary', 422),
            ('/api/render?name=invalid', 422), ('/api/render?name=CON', 404),
            ('/api/render?name=NUL.c2s', 404), ('/api/render?name=COM1', 404),
            ('/api/render?name=..%2Foutside', 404), ('/api/render?name=..%5Coutside', 404),
            ('/api/render?name=%2Fetc%2Fpasswd', 404), ('/api/render?name=C%3A%5CWindows%5Cwin.ini', 404),
            ('/api/render?name=1086_03.c2s%3A%3A%24DATA', 404), ('/api/render?name=1086_03%00', 404),
            ('/api/render?name=%252e%252e%252foutside', 404),
            ('/api/render/..%2Foutside', 404), ('/api/render/1086_03?name=invalid', 400),
            ('/api/render', 400), (base + '&format=json', 400),
            (base + '&fastattack=2', 400), (base + '&early_attack_only=2', 400),
            (base + '&div=2', 400), (base + '&zoom=NaN', 400),
            (base + '&zoom=inf', 400), (base + '&zoom=1e300', 400),
            (base + '&column=0', 400), (base + '&column=9999', 422),
            (base + '&' + marker + '=1', 400), (base + '&name=invalid', 400),
            ('/config.json', 404), ('/charts/1086_03.c2s', 404),
            ('/1086_03.c2s', 404), ('/src/service.rs', 404), ('/Cargo.toml', 404),
            ('/inspect?name=1086_03', 404), ('/_worker', 404),
        ]
        if symlinks:
            cases += [('/api/render?name=escape', 404), ('/api/render?name=alias', 404)]
        if hasattr(os, 'mkfifo'):
            cases += [('/api/render?name=pipe', 404)]
        for path, expected in cases:
            status, headers, body = request(path, {'If-None-Match': tag})
            assert status == expected, (path, status, expected, body[:200])
            assert not headers.get('ETag') and headers['Cache-Control'] == 'no-store'
            for secret in (source, marker, salt, str(root), str(chart)):
                assert secret.encode() not in body, path
        assert request('/api/render?name=' + 'x' * 3000)[0] == 414
        assert request(base, {'If-None-Match': 'x' * 9000})[0] == 431
        status, headers, body = request('/')
        assert status == 200 and b'<script src="/app.js">' in body
        assert "script-src 'self'" in headers['Content-Security-Policy']
        assert request('/app.js')[1]['Content-Type'].startswith('text/javascript')
    # Persistent configured salt keeps validators stable across process restarts.
    with server(config) as (request, _):
        assert request('/api/render?name=1086_03', {'If-None-Match': tag})[0] == 304
    settings_json = json.loads(config.read_text(encoding='utf-8'))
    settings_json['etag_salt'] = 'other-test-salt-' + '2e' * 32
    config.write_text(json.dumps(settings_json), encoding='utf-8')
    with server(config) as (request, _):
        assert image(request('/api/render?name=1086_03', {'If-None-Match': tag}))[0] != tag
    print(json.dumps({'conditional_get': 'pass', 'option_validators': 'pass', 'persistent_salt': 'pass',
                      'fastattack_api': 'pass', 'path_and_source_isolation': 'pass',
                      'symlink_rejection': 'pass' if symlinks else 'skipped (Windows symlink privilege unavailable)',
                      'request_limits': 'pass', 'security_headers': 'pass'}))
