#!/usr/bin/env python3
"""Extract only the requested release's section from the changelog."""
import argparse
from pathlib import Path
import re

parser = argparse.ArgumentParser()
parser.add_argument('tag')
args = parser.parse_args()
changelog = (Path(__file__).resolve().parents[1] / 'CHANGELOG.md').read_text(encoding='utf-8')
heading = re.search(r'^# ' + re.escape(args.tag) + r'[ \t]*$', changelog, re.MULTILINE)
if heading is None:
    parser.error(f'No changelog section for {args.tag}')
next_heading = re.search(r'^# ', changelog[heading.end():], re.MULTILINE)
end = heading.end() + next_heading.start() if next_heading else len(changelog)
section = changelog[heading.start():end].strip()
if not changelog[heading.end():end].strip():
    parser.error(f'Empty changelog section for {args.tag}')
print(section)
