#!/usr/bin/env python3
"""Snapshot reviewed Cargo notices; check/render them without a license scanner."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
LICENSES = ROOT / 'packaging/licenses'
SNAPSHOT = LICENSES / 'cargo-notices.json'
OUTPUT = LICENSES / 'CARGO_DEPENDENCY_NOTICES.md'
TARGET = 'aarch64-apple-darwin'
# These releases declare MIT but ship no copyright/license file. Preserve the
# declaration and author metadata instead of inventing a copyright statement.
METADATA_ONLY = {('dispatch', '0.2.0'), ('include_bytes_aligned', '0.1.4')}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def inputs():
    paths = [ROOT / 'Cargo.lock', ROOT / 'Cargo.toml', Path(__file__),
             LICENSES / 'about.toml', LICENSES / 'upstream.json']
    paths += sorted((ROOT / 'crates').glob('*/Cargo.toml'))
    return {str(p.relative_to(ROOT)): digest(p.read_bytes()) for p in paths}


def cargo(*args):
    return subprocess.check_output(['cargo', *args], cwd=ROOT, text=True)


def packages():
    metadata = json.loads(cargo('metadata', '--locked', '--offline', '--format-version',
                                '1', '--filter-platform', TARGET))
    tree = cargo('tree', '--locked', '--offline', '-p', 'vearth-app', '--target', TARGET,
                 '--edges', 'normal,build', '--prefix', 'none', '--format', '{p}')
    selected = {tuple(line.split()[:2]) for line in tree.splitlines()}
    # Third-party crates: published ones, and patched copies under vendor/.
    third_party = lambda p: (p['source'] or
                             Path(p['manifest_path']).is_relative_to(ROOT / 'vendor'))
    return sorted([p for p in metadata['packages'] if third_party(p) and
                   (p['name'], 'v' + p['version']) in selected],
                  key=lambda p: (p['name'], p['version'], p['source'] or ''))


def identity(p):
    return {key: p[key] for key in ('name', 'version', 'source')}


def license_files(directory):
    # Include nested vendored notices (e.g. font and sky-model data), not only
    # the license Cargo declares for a crate's own Rust source.
    for p in sorted(directory.rglob('*')):
        if not p.is_file():
            continue
        name = p.name.lower()
        if ('license' in name or 'licence' in name or
                name.startswith(('copying', 'copyright', 'notice')) or
                name in {'ofl.txt', 'ufl.txt', 'hack-regular.txt'}):
            if p.suffix not in {'.rs', '.html', '.json', '.toml'}:
                yield p


def generate(report_path):
    report = json.loads(report_path.read_text())
    pkgs = packages()
    expected = {p['id'] for p in pkgs}
    actual = {x['package']['id'] for x in report['crates']}
    if actual != expected:
        raise ValueError('cargo-about report does not match the current app dependency graph')
    upstream = json.loads((LICENSES / 'upstream.json').read_text())
    texts, entries = {}, []
    for p in pkgs:
        directory = Path(p['manifest_path']).parent
        refs = []

        def add(text, source):
            text = text.replace('\r\n', '\n').replace('\r', '\n')
            checksum = digest(text.encode())
            texts[checksum] = text
            ref = {'sha256': checksum, 'origin': source}
            if ref not in refs:
                refs.append(ref)

        files = list(license_files(directory))
        # Workspace git crates may inherit their license from the repository root.
        if (p['source'] or '').startswith('git+'):
            for parent in directory.parents:
                if (parent / '.git').exists():
                    files += [f for f in license_files(parent) if f.parent == parent]
                    break
        for path in sorted(set(files)):
            try:
                text = path.read_text()
            except UnicodeDecodeError:
                continue
            # Some archives encode a symlink as a tiny relative-path file.
            if len(text) < 100:
                target = path.parent / text.strip()
                if target.is_file():
                    text = target.read_text()
                else:
                    continue
            add(text, 'crate source: ' + os.path.relpath(path, directory))
        for license in report['licenses']:
            if license['source_path'] and any(u['crate']['id'] == p['id'] for u in license['used_by']):
                path = Path(license['source_path'])
                if digest(license['text'].encode()) not in {r['sha256'] for r in refs}:
                    add(license['text'], 'cargo-about excerpt: ' + os.path.relpath(path, directory))
        for item in upstream:
            if (item['name'], item['version']) == (p['name'], p['version']):
                if digest(item['text'].encode()) != item['sha256']:
                    raise ValueError('upstream notice checksum mismatch: ' + item['url'])
                add(item['text'], item['url'])
        note = None
        if not refs and (p['name'], p['version']) in METADATA_ONLY:
            if p['license'] != 'MIT':
                raise ValueError('reviewed metadata-only license changed')
            template = next(l['text'] for l in report['licenses'] if l['id'] == 'MIT' and not l['source_path'])
            template = template.replace('Copyright (c) <year> <copyright holders>\n\n', '')
            if '<year>' in template or '<copyright holders>' in template:
                raise ValueError('unexpected MIT template')
            add(template, 'MIT license text; license grant recorded in upstream Cargo.toml')
            note = ('This release declares MIT in Cargo.toml but supplies no separate copyright '
                    'notice or license file. No copyright statement has been fabricated.')
        if not refs:
            raise ValueError('no actual notice found for ' + p['id'])
        entry = identity(p) | {'declared_license': p['license'], 'repository': p['repository'],
                               'authors': p['authors'], 'notices': sorted(refs, key=lambda r: (r['sha256'], r['origin']))}
        if note:
            entry['note'] = note
        entries.append(entry)
    snapshot = {'target': TARGET, 'scope': 'vearth-app default features; normal and build dependencies; no dev dependencies',
                'scanner': 'cargo-about 0.9.2', 'inputs': inputs(), 'packages': entries,
                'texts': dict(sorted(texts.items()))}
    SNAPSHOT.write_text(json.dumps(snapshot, indent=2, ensure_ascii=False) + '\n')
    OUTPUT.write_text(render(snapshot))
    print(f'Generated notices for {len(entries)} crates, {len(texts)} distinct texts')


def render(snapshot):
    texts = snapshot['texts']
    numbers = {key: i for i, key in enumerate(sorted(texts), 1)}
    lines = ['# Cargo dependency notices', '',
             'Low Poly Earth for Apple silicon macOS. ' + snapshot['scope'] + '.', '',
             'The following third-party components retain their upstream terms. The application’s',
             'MIT License does not replace these terms. License alternatives and supplemental',
             'font, data, and vendored-code notices are preserved below.', '']
    for p in snapshot['packages']:
        if 'MPL-2.0' in p['declared_license']:
            if not p['source'].startswith('registry+'):
                raise ValueError('review the source-availability notice for ' + p['name'])
            lines += [f"Source for the unmodified MPL-2.0 component {p['name']} {p['version']}:",
                      f"https://crates.io/api/v1/crates/{p['name']}/{p['version']}/download .",
                      'You may obtain, modify, and redistribute that covered source under MPL-2.0;',
                      'see its notice below.', '']
    lines += ['## Components', '']
    for p in snapshot['packages']:
        lines += [f"### {p['name']} {p['version']}", '', f"Declared license: {p['declared_license']}", '',
                  f"Source: {p['source']}", '']
        if p['repository']:
            lines += [f"Upstream: {p['repository']}", '']
        if p['authors']:
            lines += ['Upstream author metadata: ' + '; '.join(p['authors']), '']
        if p.get('note'):
            lines += [p['note'], '']
        lines += [f"- [Notice {numbers[r['sha256']]}](#notice-{numbers[r['sha256']]}) — {r['origin']}" for r in p['notices']]
        lines += ['']
    lines += ['## License and copyright texts', '']
    for key in sorted(texts):
        text = '\n'.join(line.rstrip() for line in texts[key].splitlines()).rstrip()
        lines += [f'### Notice {numbers[key]}', '', '````', text, '````', '']
    return '\n'.join(lines)


def check():
    snapshot = json.loads(SNAPSHOT.read_text())
    if snapshot['inputs'] != inputs():
        raise ValueError('Cargo notice inputs changed; regenerate and review the notices (see docs/DEVELOPMENT.md)')
    if [identity(p) for p in packages()] != [identity(p) for p in snapshot['packages']]:
        raise ValueError('Cargo notice dependency coverage is stale')
    for key, text in snapshot['texts'].items():
        if digest(text.encode()) != key:
            raise ValueError('notice text checksum mismatch')
    for p in snapshot['packages']:
        if not p['notices'] or any(r['sha256'] not in snapshot['texts'] for r in p['notices']):
            raise ValueError('missing notice for ' + p['name'])
    if OUTPUT.read_text() != render(snapshot):
        raise ValueError('rendered Cargo notices do not match the snapshot')
    print(f"Cargo notices verified: {len(snapshot['packages'])} crates ({TARGET})")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report', type=Path, help='regenerate from cargo-about 0.9.2 JSON after reviewing new licenses')
    parser.add_argument('--check', action='store_true', help='verify current snapshot and rendered notices (default)')
    args = parser.parse_args()
    try:
        if args.report:
            generate(args.report)
        else:
            check()
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))


if __name__ == '__main__':
    main()
