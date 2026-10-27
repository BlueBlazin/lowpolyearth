#!/usr/bin/env python3
"""Publish only reviewed captures whose hashes and completion reports match."""
from datetime import datetime
import hashlib
from html import escape
import json
from pathlib import Path
import subprocess
from PIL import Image

SITE = Path(__file__).resolve().parents[1]
RAW = SITE / 'capture/photos/current'
# Renditions stop at 2048 px; the site serves no full-resolution originals. Quality 85
# with sharp RGB-to-YUV matches the source at viewing size; lower settings soften edges.
WIDTHS = (640, 1024, 1440, 2048)
CATEGORIES = {'Cities', 'Terrain', 'Above', 'Towns'}


def conditions(overrides):
    values = dict(part.split('=', 1) for part in overrides.split(','))
    kind = values.get('kind', 'none')
    sky = {'rain': 'Rain', 'snow': 'Snowfall'}.get(kind) or (
        'Cloud' if float(values.get('cloud', 0)) >= 0.5 else 'Clear')
    return f'{sky}, snow cover' if float(values.get('snow', 0)) >= 0.5 else sky


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def main():
    scenes = json.loads((SITE / 'capture/gallery-scenes.json').read_text())['scenes']
    binary_sha = digest(SITE.parent / 'target/release/vearth-app')
    reviewed = []
    entries = []
    for scene in scenes:
        name = scene['id']
        provenance_path = RAW / f'{name}.provenance.json'
        if not provenance_path.exists():
            continue
        provenance = json.loads(provenance_path.read_text())
        if provenance.get('binary_sha256') != binary_sha:
            continue
        if provenance.get('visual_review') != 'approved':
            continue
        source = RAW / f'{name}.png'
        if provenance['image_sha256'] != digest(source) or provenance['scene'] != scene:
            raise ValueError(f'{name}: reviewed image or scene changed')
        report = json.loads((RAW / f'{name}.json').read_text())
        if any(report['generation'][key] for key in ('missing_cells', 'in_flight_jobs', 'pending_tree_assets')):
            raise ValueError(f'{name}: incomplete scene')
        providers = [provider for provider in report['network']['providers']
                     if provider['name'].startswith('async.')
                     and not provider['name'].endswith(':protomaps-build-resolver')]
        if not providers or any(p['errors'] or p['in_flight'] or p['queued'] for p in providers):
            raise ValueError(f'{name}: scene source failed or remains pending')
        if report['render']['dropped_chunks'] or report['render_samples']['sun_shadow_capacity_drops']:
            raise ValueError(f'{name}: dropped geometry or shadow casters')
        if not report['render']['sun_shadow_configured'] or provenance['technical_checks'] != 'passed':
            raise ValueError(f'{name}: capture checks failed')
        reviewed.append((scene, provenance, report))
    if len(reviewed) < 15:
        raise ValueError(f'At least 15 reviewed captures are required; found {len(reviewed)}')
    (SITE / 'assets/gallery').mkdir(parents=True, exist_ok=True)
    (SITE / 'assets/web').mkdir(parents=True, exist_ok=True)
    # Stale renditions and legacy full-size JPGs must not linger as public assets.
    for stale in [*(SITE / 'assets/gallery').glob('*.jpg'), *(SITE / 'assets/web').glob('*-[0-9]*.webp')]:
        stale.unlink()
    for scene, provenance, report in reviewed:
        name = scene['id']
        source = RAW / f'{name}.png'
        with Image.open(source) as image:
            if image.mode == 'RGBA' and image.getchannel('A').getextrema() != (255, 255):
                raise ValueError(f'{name}: screenshot contains transparent pixels')
        for width in WIDTHS:
            subprocess.run(['cwebp', '-quiet', '-q', '85', '-m', '6', '-sharp_yuv', '-resize', str(width), '0', str(source),
                            '-o', str(SITE / 'assets/web' / f'{name}-{width}.webp')], check=True)
        entries.append({**scene, 'width': report['surface']['width'], 'height': report['surface']['height'],
                        'file': f'assets/web/{name}-{WIDTHS[-1]}.webp',
                        'sha256': digest(SITE / 'assets/web' / f'{name}-{WIDTHS[-1]}.webp'),
                        'source_png_sha256': provenance['image_sha256'],
                        'binary_sha256': provenance['binary_sha256'],
                        'wanted_cells': report['generation']['wanted_cells'],
                        'missing_cells': 0, 'pending_jobs': 0, 'pending_tree_assets': 0,
                        'scene_source_errors': 0, 'dropped_geometry': 0, 'shadow_capacity_drops': 0,
                        'shadow_caster_chunks_max': report['render_samples']['sun_shadow_caster_chunks']['max'],
                        'visual_review': 'approved'})
    (SITE / 'assets/gallery/gallery.json').write_text(json.dumps({
        'schema_version': 1, 'renderer': 'continuous-surface',
        'binary_sha256': binary_sha, 'screenshots': entries,
    }, ensure_ascii=False, indent=2)+'\n')
    figures = []
    for number, entry in enumerate(entries, 1):
        name = entry['id']; place = escape(entry['place']); country = escape(entry['country'])
        if entry['category'] not in CATEGORIES:
            raise ValueError(f"{name}: unknown category {entry['category']!r}")
        title = place if place == country else f'{place}, {country}'
        date = datetime.fromisoformat(entry['time'].replace('Z', '+00:00')).strftime('%-d %B %Y, %H:%M UTC')
        meta = f"{country}. {entry['angle']}. {date}. {conditions(entry['overrides'])}."
        # Every fifth photograph spans the full row; the rest pair up.
        wide = number % 5 == 1
        sizes = '100vw' if wide else '(max-width: 700px) 100vw, 50vw'
        srcset = ', '.join(f'assets/web/{name}-{width}.webp {width}w' for width in WIDTHS)
        figures.append(f'''        <figure{' class="wide"' if wide else ''} data-category="{entry['category'].lower()}">
          <a class="photo-link" href="{entry['file']}" data-photo data-title="{place}" data-meta="{escape(meta)}">
            <img src="assets/web/{name}-1440.webp" srcset="{srcset}" sizes="{sizes}" width="3840" height="2160" {'fetchpriority="high"' if number == 1 else 'loading="lazy"'} decoding="async" alt="{escape(entry['angle'])}, {title}. Rendered in Low Poly Earth.">
          </a>
          <figcaption><h2>{place}</h2><p>{country}. {escape(entry['angle'])}.</p></figcaption>
        </figure>''')
    template = (SITE / 'capture/screenshots-template.html').read_text()
    (SITE / 'screenshots.html').write_text(template.replace('{{COUNT}}',str(len(entries))).replace('{{FIGURES}}','\n'.join(figures)))
    print(f'Published {len(entries)} reviewed screenshots.')


if __name__ == '__main__':
    main()
