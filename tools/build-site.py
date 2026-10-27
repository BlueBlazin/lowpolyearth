#!/usr/bin/env python3
"""Validate and assemble the static website in target/site for hosting."""

import hashlib
from html.parser import HTMLParser
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]
SITE = ROOT / 'site'
OUTPUT = ROOT / 'target/site'
# Absolute origin for social-preview metadata, which crawlers do not resolve relatively.
ORIGIN = 'https://lowpolyearth.online'
# Everything served from these prefixes is content-addressed, so it may be cached forever.
IMMUTABLE_PREFIXES = ('assets/', 'fonts/')
URL_ATTRIBUTE = re.compile(r'''\b(href|src|poster|srcset|imagesrcset|content)="([^"]*)"''')
CSS_URL = re.compile(r'''url\(([\s'"]*)([^)'"\s]+)([\s'"]*)\)''')


class Page(HTMLParser):
    def __init__(self, path):
        super().__init__()
        self.path = path
        self.links = []
        self.ids = set()
        self.feed(path.read_text())

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if 'id' in attrs:
            if attrs['id'] in self.ids:
                raise ValueError(f'{self.path.name}: duplicate id {attrs["id"]}')
            self.ids.add(attrs['id'])
        for key in ('href', 'src', 'poster', 'data-src', 'data-image'):
            if attrs.get(key):
                self.links.append(attrs[key])
        # Social-preview images are published even when no visible element uses them.
        if tag == 'meta' and attrs.get('property') == 'og:image' and attrs.get('content'):
            self.links.append(attrs['content'])
        for key in ('srcset', 'imagesrcset'):
            for source in attrs.get(key, '').split(','):
                if source.strip():
                    self.links.append(source.split()[0])


def public_path(source, value):
    url = urlsplit(value)
    if url.scheme or url.netloc:
        return None
    path = unquote(url.path)
    target = (SITE / path.lstrip('/') if path.startswith('/')
              else source.parent / path if path else source)
    if target.is_dir():
        target /= 'index.html'
    target = target.resolve()
    if not target.is_relative_to(SITE) or not target.is_file():
        raise ValueError(f'{source.relative_to(SITE)}: missing or invalid local asset {value}')
    return target


def build():
    subprocess.run([sys.executable, str(ROOT / 'tools/write-privacy-policy.py'),
                    '--site', '--check', str(SITE / 'privacy.html')], check=True)
    pages = {p.resolve(): Page(p) for p in SITE.glob('*.html')}
    files = set(pages)
    files.add(SITE / '_headers')
    for source, page in pages.items():
        for link in page.links:
            target = public_path(source, link)
            if target is None:
                continue
            files.add(target)
            anchor = unquote(urlsplit(link).fragment)
            if anchor and target in pages and anchor not in pages[target].ids:
                raise ValueError(f'{source.name}: missing anchor {link}')
    # Follow local CSS imports and font/image references if they are introduced later.
    pending = [p for p in files if p.suffix == '.css']
    while pending:
        source = pending.pop()
        for value in re.findall(r'url\([\s\'"]*([^\)\'"\s]+)', source.read_text()):
            target = public_path(source, value)
            if target is not None and target not in files:
                files.add(target)
                if target.suffix == '.css':
                    pending.append(target)
    manifest_path = SITE / 'assets/gallery/gallery.json'
    manifest = json.loads(manifest_path.read_text())
    if manifest.get('renderer') != 'continuous-surface' or not manifest.get('binary_sha256'):
        raise ValueError('Publish a reviewed continuous-surface gallery before building')
    for entry in manifest['screenshots']:
        if entry['binary_sha256'] != manifest['binary_sha256'] or entry['visual_review'] != 'approved':
            raise ValueError('Gallery contains unreviewed or mixed-renderer screenshots')
        path = SITE / entry['file']
        if hashlib.sha256(path.read_bytes()).hexdigest() != entry['sha256']:
            raise ValueError(f'Published screenshot does not match provenance: {path.name}')
    files.add(manifest_path)
    # The flight film is optional; when published it must match the gallery's renderer.
    flight_path = SITE / 'assets/web/virtual-earth-flight.json'
    if flight_path.exists():
        flight = json.loads(flight_path.read_text())
        if flight['visual_review'] != 'approved' or flight['binary_sha256'] != manifest['binary_sha256']:
            raise ValueError('Flight and gallery must be reviewed captures from the same renderer')
        for name, expected in flight['files'].items():
            path = public_path(flight_path, name)
            if path is None or hashlib.sha256(path.read_bytes()).hexdigest() != expected:
                raise ValueError(f'Flight asset does not match provenance: {name}')
            files.add(path)
        files.add(flight_path)
    for path in files:
        if not path.is_file():
            raise ValueError(f'Missing public file: {path}')
        if path.stat().st_size > 25 * 1024 * 1024:
            raise ValueError(f'File exceeds Cloudflare Pages 25 MiB limit: {path}')
    # Content-address every static asset so it can be cached as immutable; only
    # the HTML entry points (fixed names, revalidated) point at the current set.
    renamed = {}
    for path in files:
        relative = path.relative_to(SITE)
        if path.suffix == '.html' or relative.as_posix() in ('_headers', 'favicon.svg'):
            continue
        if relative.as_posix() == 'assets/gallery/gallery.json':
            renamed[path] = Path('gallery.json')
            continue
        tag = hashlib.sha256(path.read_bytes()).hexdigest()[:10]
        folder = (relative.parent if (relative.parent.as_posix() + '/').startswith(IMMUTABLE_PREFIXES)
                  else Path('assets'))
        renamed[path] = folder / f'{path.stem}.{tag}{path.suffix}'

    def rewrite(source, value, absolute_origin=False):
        parts = []
        for token in value.split(','):
            stripped = token.strip()
            url, _, descriptor = stripped.partition(' ')
            target = public_path(source, url) if url else None
            if target in renamed:
                new = renamed[target].as_posix()
                if absolute_origin:
                    new = f'{ORIGIN}/{new}'
                elif url.startswith('/') or source.suffix == '.css':
                    new = '/' + new
                stripped = new + (' ' + descriptor if descriptor else '')
            parts.append(token.replace(token.strip(), stripped) if token.strip() else token)
        return ','.join(parts)

    def rewrite_html(source):
        def replace(match):
            name, value = match.groups()
            if name == 'content':
                if not value.startswith(('assets/', '/assets/')):
                    return match.group(0)
                return f'{name}="{rewrite(source, value, absolute_origin=True)}"'
            if name in ('srcset', 'imagesrcset'):
                return f'{name}="{rewrite(source, value)}"'
            url = urlsplit(value)
            if url.scheme or url.netloc or not url.path:
                return match.group(0)
            return f'{name}="{rewrite(source, value)}"'
        return URL_ATTRIBUTE.sub(replace, source.read_text())

    def rewrite_css(source):
        return CSS_URL.sub(lambda m: f'url({m.group(1)}{rewrite(source, m.group(2))}{m.group(3)})',
                           source.read_text())

    # Replace only this script's generated output after every input has passed validation.
    if OUTPUT.is_symlink():
        raise ValueError(f'Output must not be a symlink: {OUTPUT}')
    if OUTPUT.exists():
        shutil.rmtree(OUTPUT)
    OUTPUT.mkdir(parents=True)
    for path in sorted(files):
        destination = OUTPUT / renamed.get(path, path.relative_to(SITE))
        destination.parent.mkdir(parents=True, exist_ok=True)
        if path.suffix == '.html':
            destination.write_text(rewrite_html(path))
        elif path.suffix == '.css':
            destination.write_text(rewrite_css(path))
        else:
            shutil.copy2(path, destination)
    with (OUTPUT / '_headers').open('a') as headers:
        for prefix in IMMUTABLE_PREFIXES:
            headers.write(f'\n/{prefix}*\n  Cache-Control: public, max-age=31536000, immutable\n')
    size = sum(p.stat().st_size for p in files)
    print(f'Validated {len(pages)} pages and their local links, anchors, and screenshot hashes.')
    print(f'Created {OUTPUT}: {len(files)} files, {size / 1024 / 1024:.1f} MiB')


if __name__ == '__main__':
    try:
        build()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        raise SystemExit(str(error)) from error
