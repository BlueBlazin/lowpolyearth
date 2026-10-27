#!/usr/bin/env python3
"""Render mapped-world review cameras using the game's normal screenshot path."""
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
    parser.add_argument('--cameras', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--references', type=Path)
    parser.add_argument('--timeout', type=int, default=120)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    binary = args.binary.resolve()
    camera_bytes = args.cameras.read_bytes()
    identity = {'binary_sha256':hashlib.file_digest(binary.open('rb'),'sha256').hexdigest(),
                'input_sha256':hashlib.sha256(camera_bytes).hexdigest()}
    run_path = args.output/'capture-run.json'
    if run_path.exists() and json.loads(run_path.read_text()) != identity:
        raise ValueError('Output contains a different capture revision; choose a new output directory.')
    resume = run_path.exists()
    run_path.write_text(json.dumps(identity,indent=2)+'\n')
    (args.output/'cameras.json').write_bytes(camera_bytes)
    references = {r['key']:r for r in json.loads(args.references.read_text())} if args.references else {}
    records=[]
    for camera in json.loads(camera_bytes):
        key=camera['key']; path=(args.output/f'{key}.png').resolve(); report=args.output/f'{key}.json'
        pose=','.join(str(camera[k]) for k in ('lat','lon','alt','heading','pitch'))
        command=[str(binary),'--screenshot',str(path),'--at',pose,'--size','1000x750','--timeout',str(args.timeout),
                 '--time',f'2026-09-15T{camera["utc_hour"]:02d}:00Z','--weather','clear','--screenshot-report',str(report.resolve())]
        if not resume or not path.exists() or not report.exists():
            with (args.output/f'{key}.log').open('w') as log:
                result=subprocess.run(command,stdout=log,stderr=log,timeout=args.timeout+60)
            if result.returncode: print(f'{key}: capture exit {result.returncode}',flush=True)
        if not path.exists() or not report.exists():
            records.append(dict(camera,status='capture failed',command=command));continue
        data=json.loads(report.read_text()); generation=data['generation']
        complete=all(generation[k]==0 for k in ('missing_cells','in_flight_jobs','pending_tree_assets'))
        records.append(dict(camera,status='loaded' if complete else 'incomplete streaming',image=str(path),
                            report=str(report.resolve()),camera_agl_m=data['pose']['camera_agl_m'],command=command,
                            reference=references.get(key)))
        write_gallery(args.output,records)
        print(f'{key}: {records[-1]["status"]}, AGL {data["pose"]["camera_agl_m"]:.1f} m',flush=True)
    manifest={'binary':str(binary),'binary_sha256':hashlib.file_digest(binary.open('rb'),'sha256').hexdigest(),
              'camera_source_sha256':identity['input_sha256'],'entries':records}
    (args.output/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    write_gallery(args.output,records)


def write_gallery(output,records):
    captured=[r for r in records if 'image' in r]
    sheet=Image.new('RGB',(1600,max(1,(len(captured)+3)//4)*330),'#ecebe6');draw=ImageDraw.Draw(sheet)
    esc=html.escape; sections=[]; markdown=['# In-game world architecture review','',
        'Actual mapped towns and cities, rendered through normal streaming and lighting. User visual approval is pending.','']
    for i,r in enumerate(captured):
        x,y=(i%4)*400,(i//4)*330
        im=Image.open(r['image']).convert('RGB');im.thumbnail((400,300));sheet.paste(im,(x,y))
        draw.text((x+7,y+303),r['key']+' / '+r['status'],fill='#24332c')
        sections.append(f'<section><h2>{esc(r["label"])}</h2><p>{esc(r["status"])} · camera {r["camera_agl_m"]:.0f} m above terrain</p><a href="{esc(Path(r["image"]).name)}"><img loading="lazy" src="{esc(Path(r["image"]).name)}"></a>')
        markdown += [f'## {r["label"]}','',f'![{r["label"]}]({r["image"]})','']
        if r.get('review_note'):
            sections.append(f'<p><strong>Review limitation:</strong> {esc(r["review_note"])}</p>')
            markdown += [f'**Review limitation:** {r["review_note"]}', '']
        if r.get('reference'):
            ref=r['reference'];sections.append(f'<p><a href="{esc(ref["reference_page"])}">Photographic reference</a> — {esc(ref["observed"])}</p>')
            markdown += [f'[Photographic reference]({ref["reference_page"]}) — {ref["observed"]}','']
        sections.append('</section>')
    for r in records:
        if 'image' not in r:
            sections.append(f'<section><h2>{esc(r["label"])}</h2><p>{esc(r["status"])}</p></section>')
            markdown += [f'## {r["label"]}', '', r['status'], '']
    if not captured:
        draw.text((20,20),'No captures succeeded; see capture logs.',fill='#24332c')
    sheet.save(output/'world-contact.jpg',quality=94)
    header='''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Vearth · in-game world review</title><style>body{max-width:1100px;margin:32px auto;padding:0 24px;font:16px/1.5 system-ui;background:#f3f2ee;color:#25342d}img{width:100%}section{border-top:1px solid #bbc3bd;margin-top:36px;padding-top:20px}a{color:#246447}</style><h1>In-game world architecture review</h1><p>Actual mapped towns and cities with terrain, streets, vegetation and surrounding buildings, rendered through normal streaming and lighting. Each capture includes its camera and loading report. Loaded means requested cells completed; it does not establish complete building-source coverage or visual acceptance. User approval remains pending.</p>'''
    (output/'index.html').write_text(header+''.join(sections)+'</html>')
    (output/'WORLD_GALLERY.md').write_text('\n'.join(markdown))


if __name__=='__main__':main()
