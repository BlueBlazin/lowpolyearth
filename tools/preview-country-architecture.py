#!/usr/bin/env python3
"""Export weighted source palettes and evidence; not a rendering simulation."""
import argparse
import html
import importlib.util
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / 'crates/vearth-gen/assets/architecture'


def esc(value):
    return html.escape(str(value), quote=True)


def palette(label, choices):
    if not choices:
        return f'<p>{esc(label)}: not specified for this stock.</p>'
    total = sum(c['weight'] for c in choices)
    chips = ''.join(f'<span style="flex:{c["weight"]};background:rgb({",".join(map(str,c["color"]))})" '
                    f'title="{esc(c.get("material",label))}; variant {c.get("variant",0)}; relative weight {c["weight"]}/{total}"></span>'
                    for c in choices)
    names = ' · '.join(f'{esc(c.get("material", "pigment"))} v{c.get("variant",0)} {esc(c["color"])} ×{c["weight"]}' for c in choices)
    return f'<div class="palette"><strong>{esc(label)}</strong><div class="bar">{chips}</div><small>{names}</small></div>'


def entry_view(entry, heading, compiler):
    parts = [f'<section data-search="{esc(heading.lower())}"><h2>{esc(heading)}</h2>',
             f'<p class="status">{esc(entry["review_status"])}</p><p>{esc(entry["label"])}</p><p>{esc(entry["scope"])}</p>']
    for role in ('urban', 'town', 'rural'):
        parts.append(f'<h3>{role}</h3>')
        for choice in entry[role]:
            archetype = compiler.archetype_data(entry, choice, role)
            parts.append(f'<details><summary>{esc(archetype["name"])} ×{choice["weight"]}</summary>')
            parts.append('<p>Roof shape: '+ ' · '.join(f'{esc(s["shape"])} ×{s["weight"]}' for s in archetype['roof_shapes'])+'</p>')
            walls = [dict(w, material=w.get('material', archetype['wall_material'])) for w in archetype['walls']]
            parts.append(palette('walls', walls))
            for finish_role in ('roofs', 'flat_roofs', 'trim', 'joinery'):
                parts.append(palette(finish_role, archetype[finish_role]))
            parts.append('</details>')
    parts.append('<details><summary>Evidence and limits</summary>')
    for role, basis in entry['weight_basis'].items():
        parts.append(f'<p><strong>{esc(role)}</strong>: {esc(basis)}</p>')
    for source in entry['evidence']:
        parts.append(f'<p><a href="{esc(source["url"])}">{esc(source["title"])}</a> — {esc(source["scope"])}. {esc(source["note"])}</p>')
    for source in entry.get('visual_references', []):
        parts.append(f'<p><a href="{esc(source["url"])}">Inspected reference</a>: {esc(source["scope"])}. {esc(source["observed"])}</p>')
    parts.append('</details></section>')
    return ''.join(parts)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=ROOT/'output/architecture-review/country-catalog.html')
    args = parser.parse_args()
    spec = importlib.util.spec_from_file_location('country_build', Path(__file__).with_name('build-country-architecture.py'))
    compiler = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(compiler)
    compiler.build(check=True)
    entries = sorted([e for p in (ASSETS/'countries').glob('*.json') for e in json.loads(p.read_text())], key=lambda e:e['country'])
    body = []
    for entry in entries:
        body.append(entry_view(entry, entry['country'], compiler))
        for region in entry.get('regions', []):
            body.append(entry_view(compiler.region_profile(entry, region), entry['country']+' / '+region['label'], compiler))
    page = '''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Vearth · catalog evidence</title><style>
body{max-width:1100px;margin:auto;padding:32px;font:16px/1.5 system-ui;background:#f4f3ee;color:#25332e}h1{font-size:36px}section{padding:24px 0;border-top:1px solid #b5bdb5}p{max-width:950px}.status{font-weight:700}.palette{margin:18px 0}.bar{display:flex;height:36px;border:1px solid #aaa;gap:2px;margin:4px 0}.bar span{min-width:4px}small{color:#56615a}a{color:#236244}input{padding:12px;font:inherit;width:90%;margin:16px 0}summary{cursor:pointer}header{position:sticky;top:0;background:#f4f3ee;padding:10px 0}
</style><h1>Catalog evidence and weighted palettes</h1><p>Source data, not an in-game preview. Bar widths show authored relative weights, not measured census shares. Review status describes evidence work; rendered appearance still needs user approval.</p><header><input type="search" aria-label="Filter entries" placeholder="Filter country or subdivision" oninput="document.querySelectorAll('section').forEach(s=>s.hidden=!s.dataset.search.includes(this.value.toLowerCase()))"></header>'''
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(page+''.join(body)+'</html>')
    print(args.output.resolve())


if __name__ == '__main__':
    main()
