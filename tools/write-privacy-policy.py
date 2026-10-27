#!/usr/bin/env python3
"""Render the shared app policy for offline packaging or the static website."""
import argparse
import html
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def policy_blocks():
    blocks = []
    for paragraph in (ROOT / 'docs/PRIVACY.md').read_text().strip().split('\n\n'):
        if paragraph.startswith('## '):
            tag, text = 'h2', paragraph[3:]
        elif paragraph.startswith('# '):
            tag, text = 'h1', paragraph[2:]
        else:
            tag, text = 'p', paragraph.replace('\n', ' ')
        blocks.append((tag, text))
    return blocks


def render():
    blocks = [f'<{tag}>{html.escape(text)}</{tag}>' for tag, text in policy_blocks()]
    return ('<!doctype html>\n<html lang="en"><head><meta charset="utf-8">\n'
            '<meta name="viewport" content="width=device-width, initial-scale=1">\n'
            '<title>Low Poly Earth Privacy Policy</title>\n'
            '<style>body{font:17px/1.65 system-ui,sans-serif;max-width:46rem;'
            'margin:3rem auto;padding:0 1.5rem;color:#20241f;background:#fafbf8}'
            'h1{line-height:1.2}h2{font-size:1.25rem;margin-top:2rem}</style>\n'
            '</head><body><main>\n' + '\n'.join(blocks) + '\n</main></body></html>\n')


def render_site():
    header, contents, sections = [], [], []
    for tag, text in policy_blocks():
        escaped = html.escape(text)
        if tag == 'h2':
            anchor = re.sub(r'[^a-z0-9]+', '-', text.lower()).strip('-')
            contents.append(f'<li><a href="#{anchor}">{escaped}</a></li>')
            if sections:
                sections.append('</section>')
            sections.append(f'<section aria-labelledby="{anchor}">\n<h2 id="{anchor}">{escaped}</h2>')
        elif not sections:
            header.append(f'<{tag}>{escaped}</{tag}>')
        else:
            sections.append(f'<{tag}>{escaped}</{tag}>')
    if sections:
        sections.append('</section>')
    template = (ROOT / 'tools/templates/privacy.html').read_text()
    return (template.replace('{{HEADER}}', '\n'.join(header))
            .replace('{{CONTENTS}}', '\n'.join(contents))
            .replace('{{POLICY}}', '\n'.join(sections)))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--site', action='store_true', help='use the website layout')
    parser.add_argument('--check', action='store_true', help='verify the output without writing it')
    args = parser.parse_args()
    rendered = render_site() if args.site else render()
    if args.check:
        if not args.output.is_file() or args.output.read_text() != rendered:
            parser.exit(1, f'{args.output} is missing or stale; regenerate it with this command without --check\n')
        print(f'Privacy policy verified: {args.output}')
    else:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered)
