#!/usr/bin/env python3
"""Capture a review gallery with the actual game executable and resolver specimens.

Generate specimens with `cargo run -q -p vearth-gen --example architecture_review`.
All images are renderer output; contact sheets only resize and label those images.
"""
import argparse
import hashlib
import html
import json
from pathlib import Path
import subprocess
from PIL import Image, ImageDraw


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--specimens', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--references', type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    binary = args.binary.resolve()
    identity = {'binary_sha256':hashlib.file_digest(binary.open('rb'),'sha256').hexdigest(),
                'input_sha256':hashlib.sha256(args.specimens.read_bytes()).hexdigest()}
    run_path = args.output/'capture-run.json'
    if run_path.exists() and json.loads(run_path.read_text()) != identity:
        raise ValueError('Output contains a different capture revision; choose a new output directory.')
    resume = run_path.exists()
    run_path.write_text(json.dumps(identity,indent=2)+'\n')
    metadata = json.loads(args.specimens.read_text())
    references = {e['key']:e for e in json.loads(args.references.read_text())} if args.references else {}
    records = []
    for entry in metadata['entries']:
        images = {}
        for view, stage in [('distribution', entry['stage']), ('detail', entry['detail_stage'])]:
            image = (args.output / f'{entry["key"]}-{view}.png').resolve()
            command = [str(binary), '--stage', stage, '--screenshot', str(image), '--size', '1000x750',
                       '--stage-sun-hour', '12', '--stage-elevation', '38']
            if not resume or not image.exists():
                result = subprocess.run(command, capture_output=True, timeout=120)
                if result.returncode:
                    raise RuntimeError(f'{stage}: {result.stderr.decode()}')
            images[view] = str(image)
        records.append(dict(entry, images=images, reference=references.get(entry['key'])))
        print(entry['key'], flush=True)
    manifest = {'binary':str(binary), 'binary_sha256':hashlib.file_digest(binary.open('rb'), 'sha256').hexdigest(),
                'gen_version':metadata['gen_version'], 'size':[1000,750], 'sun_hour':12, 'elevation':38,
                'specimens_sha256':hashlib.sha256(args.specimens.read_bytes()).hexdigest(), 'entries':records}
    (args.output/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
    for view in ('distribution', 'detail'):
        sheet = Image.new('RGB', (1600, ((len(records)+3)//4)*330), '#edece7')
        draw = ImageDraw.Draw(sheet)
        for i, entry in enumerate(records):
            x,y = (i%4)*400,(i//4)*330
            im = Image.open(entry['images'][view]).convert('RGB')
            im.thumbnail((400,300))
            sheet.paste(im,(x,y))
            draw.text((x+8,y+303), entry['key']+' / '+entry['region'], fill='#172522')
        sheet.save(args.output/f'{view}-contact.jpg', quality=92)
    esc = html.escape
    sections = []
    for entry in records:
        section = f'<section><h2>{esc(entry["label"])}</h2><p>{esc(entry["region"])} · {esc(entry["settlement"])} source context</p><div class="images">'
        for view,path in entry['images'].items():
            section += f'<figure><a href="{esc(Path(path).name)}"><img loading="lazy" src="{esc(Path(path).name)}"></a><figcaption>{esc(view)}</figcaption></figure>'
        section += '</div>'
        ref=entry['reference']
        if ref:
            section += f'<p><a href="{esc(ref["reference_page"])}">Photographic reference</a> — {esc(ref["observed"])}</p>'
        sources={s['source'] for s in entry['samples']}
        section += '<details><summary>Selected catalog entries</summary><pre>'+esc('\n'.join(sorted(sources)))+'</pre></details></section>'
        sections.append(section)
    header='''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Vearth · in-game architecture review</title><style>
body{margin:24px auto;max-width:1300px;padding:0 24px;font:16px/1.5 system-ui;color:#24342d;background:#f2f1ec}.images{display:grid;grid-template-columns:1fr 1fr;gap:12px}img{width:100%}figure{margin:0}section{border-top:1px solid #bac2bc;margin-top:32px;padding-top:20px}a{color:#246447}pre{white-space:pre-wrap}h1{font-size:38px}@media(max-width:700px){.images{grid-template-columns:1fr}}
</style><h1>In-game architecture review</h1><p>Thirty locations through the actual production resolver and renderer: 36 varied source specimens per distribution, plus four close-up specimens. Sample counts are not limits on catalog alternatives. These controlled arrangements are not reconstructions of mapped towns; mapped-world captures are provided separately. Reference observations describe coarse colors and materials, not calibrated RGB. Approval remains pending.</p>'''
    (args.output/'index.html').write_text(header+''.join(sections)+'</html>')
    print(args.output.resolve()/'index.html')


if __name__ == '__main__':
    main()
