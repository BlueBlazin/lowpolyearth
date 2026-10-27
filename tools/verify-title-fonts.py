#!/usr/bin/env python3
"""Verify full arrival fonts and licenses; --fetch restores their pinned bytes."""
import argparse
import hashlib
import json
from pathlib import Path
import urllib.parse
import urllib.request


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--fetch', action='store_true')
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1] / 'crates/vearth-app/assets/fonts/titles'
    manifest = json.loads((root / 'manifest.json').read_text())
    expected = {'manifest.json'}
    for item in manifest['files']:
        path = root / item['file']
        expected.add(item['file'])
        if args.fetch:
            url = f"https://raw.githubusercontent.com/google/fonts/{manifest['revision']}/" + urllib.parse.quote(item['source'])
            request = urllib.request.Request(url, headers={'User-Agent': 'Vearth-title-font-verifier'})
            with urllib.request.urlopen(request, timeout=90) as response:
                data = response.read()
            if hashlib.sha256(data).hexdigest() != item['sha256']:
                raise SystemExit(f'Upstream hash mismatch: {item["source"]}')
            path.write_bytes(data)
        if hashlib.sha256(path.read_bytes()).hexdigest() != item['sha256']:
            raise SystemExit(f'Hash mismatch: {path}')
    actual = {path.name for path in root.iterdir() if path.is_file()}
    if actual != expected:
        raise SystemExit(f'Unmanifested/missing files: {actual ^ expected}')
    fonts = [f for f in manifest['files'] if f['file'].endswith('.ttf')]
    families = {f['source'].split('/')[1] for f in fonts}
    for family in families:
        if f'{family}-OFL.txt' not in expected:
            raise SystemExit(f'Missing license: {family}')
    registry = (root.parents[2] / 'src/location_title/fonts.rs').read_text()
    for font in fonts:
        if font['file'] not in registry:
            raise SystemExit(f'Unused font: {font["file"]}')
    print(f'Verified {len(fonts)} full fonts, {len(families)} licenses, and renderer references.')


if __name__ == '__main__':
    main()
