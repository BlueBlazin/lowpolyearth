#!/usr/bin/env python3
"""Capture a settled Dolomites flight; publish only after visual review."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]
FRAMES = ROOT / 'site/capture/frames/virtual-earth-flight'
RECORD = FRAMES / 'provenance.json'
BINARY = ROOT / 'target/release/vearth-app'
COUNT = 145
FPS = 24


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--publish', action='store_true')
    args = parser.parse_args()
    FRAMES.mkdir(parents=True, exist_ok=True)
    if not args.publish:
        RECORD.unlink(missing_ok=True)
        for frame in FRAMES.glob('frame-*.png'):
            frame.unlink()
        command = [str(BINARY), '--capture-fly-to', '46.507,11.875,2900',
                   '--at', '46.5,11.85,2900,70,79', '--capture-end-heading', '70',
                   '--capture-end-pitch', '79', '--capture-hold-frames', '12',
                   '--capture-dir', str(FRAMES), '--capture-frames', str(COUNT),
                   '--size', '1440x810', '--timeout', '120',
                   '--time', '2026-09-12T15:00:00Z', '--weather', 'procedural',
                   '--weather-seed', '2953', '--weather-override',
                   'temperature=9,humidity=.5,cloud=.1,cloud_base=3400,cloud_top=4300,precip=0,kind=none,snow=.05,vegetation=.6,aerosol=.04,visibility=80000']
        with (FRAMES / 'capture.log').open('w') as log:
            subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=True)
        frames = sorted(FRAMES.glob('frame-*.png'))
        if len(frames) != COUNT:
            raise ValueError('Incomplete flight')
        RECORD.write_text(json.dumps({'binary_sha256': digest(BINARY), 'command': command,
                                     'fps': FPS, 'visual_review': 'pending',
                                     'frames': {p.name: digest(p) for p in frames}}, indent=2)+'\n')
        print(f'{COUNT} frames ready for visual review: {FRAMES}')
        return
    record = json.loads(RECORD.read_text())
    if record['visual_review'] != 'approved' or record['binary_sha256'] != digest(BINARY):
        raise ValueError('Review captures from the current binary before publishing')
    if len(record['frames']) != COUNT or any(digest(FRAMES / name) != sha for name, sha in record['frames'].items()):
        raise ValueError('Reviewed flight frames changed')
    output = ROOT / 'site/assets/web/virtual-earth-flight.mp4'
    subprocess.run(['ffmpeg', '-hide_banner', '-loglevel', 'warning', '-y',
                    '-framerate', str(FPS), '-i', str(FRAMES / 'frame-%04d.png'),
                    '-an', '-c:v', 'libx264', '-crf', '20', '-pix_fmt', 'yuv420p',
                    '-movflags', '+faststart', str(output)], check=True)
    poster = ROOT / 'site/assets/web/virtual-earth-flight.webp'
    subprocess.run(['cwebp', '-quiet', '-q', '82', str(FRAMES / 'frame-0072.png'), '-o', str(poster)], check=True)
    manifest = {k: record[k] for k in ('binary_sha256', 'command', 'fps', 'frames', 'visual_review')}
    manifest['command'] = [arg.replace(str(ROOT) + '/', '') for arg in manifest['command']]
    manifest['files'] = {p.name: digest(p) for p in (output, poster)}
    (ROOT / 'site/assets/web/virtual-earth-flight.json').write_text(json.dumps(manifest, indent=2)+'\n')
    print(f'Published {output}')


if __name__ == '__main__':
    main()
