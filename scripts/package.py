#!/usr/bin/env python3
"""Package only approved runtime files; never charts or a live configuration."""
import argparse
import os
from pathlib import Path
import tarfile
import tomllib
import zipfile

parser = argparse.ArgumentParser()
parser.add_argument('--platform', choices=['linux', 'windows'], required=True)
parser.add_argument('--binary', type=Path, required=True)
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
version = tomllib.loads((root / 'Cargo.toml').read_text(encoding='utf-8'))['package']['version']
if os.getenv('GITHUB_REF_TYPE') == 'tag':
    assert os.environ['GITHUB_REF_NAME'] == 'v' + version, 'Tag/package version mismatch'
assert args.binary.is_file(), 'Compiled binary missing'
files = {name: root / name for name in ('README.md', 'LICENSE', 'NOTICE', 'SECURITY.md', 'CHANGELOG.md', 'config.example.json')}
files['FONT-LICENSE'] = root / 'assets/FONT-LICENSE'
for name in ('docs/c2s-scroll-slide.md', 'docs/hold-head-and-tail.md'):
    files[name] = root / name
files['chuni-chart-rs' + ('.exe' if args.platform == 'windows' else '')] = args.binary
if args.platform == 'linux':
    files['deploy/chuni-chart.service'] = root / 'deploy/chuni-chart.service'
dist = root / 'dist'
dist.mkdir(exist_ok=True)
if args.platform == 'windows':
    output = dist / 'chuni-chart-rs-windows-x64.zip'
    with zipfile.ZipFile(output, 'w', compression=zipfile.ZIP_DEFLATED) as archive:
        for name, path in files.items():
            archive.write(path, name)
    with zipfile.ZipFile(output) as archive:
        assert sorted(archive.namelist()) == sorted(files)
else:
    output = dist / 'chuni-chart-rs-linux-x64.tar.gz'
    with tarfile.open(output, 'w:gz') as archive:
        for name, path in files.items():
            archive.add(path, arcname=name, recursive=False)
    with tarfile.open(output) as archive:
        assert sorted(archive.getnames()) == sorted(files)
print(output)
