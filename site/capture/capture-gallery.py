#!/usr/bin/env python3
"""Capture the gallery without publishing unreviewed images."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]
SCENES = Path(__file__).with_name('gallery-scenes.json')
OUT = ROOT / 'site/capture/photos/current'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--only', nargs='+', help='Scene IDs to capture')
    parser.add_argument('--timeout', type=int, default=300)
    args = parser.parse_args()
    specification = json.loads(SCENES.read_text())
    unknown = set(args.only or []) - {scene['id'] for scene in specification['scenes']}
    if unknown:
        parser.error('Unknown scene IDs: ' + ', '.join(sorted(unknown)))
    binary = ROOT / 'target/release/vearth-app'
    binary_sha = hashlib.file_digest(binary.open('rb'), 'sha256').hexdigest()
    OUT.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env['RUST_LOG'] = 'info'
    env.pop('VEARTH_DISABLE_SUN_SHADOWS', None)
    failures = []
    for scene in specification['scenes']:
        name = scene['id']
        if args.only and name not in args.only:
            continue
        image = OUT / f'{name}.png'
        report = OUT / f'{name}.json'
        # Failed retries must not leave an earlier image looking current.
        image.unlink(missing_ok=True)
        report.unlink(missing_ok=True)
        (OUT / f'{name}.provenance.json').unlink(missing_ok=True)
        command = [str(binary), '--screenshot', str(image), '--screenshot-report', str(report),
                   '--fresh-chunks', '--size', f"{specification['width']}x{specification['height']}",
                   '--at', ','.join(map(str, scene['pose'])), '--time', scene['time'],
                   '--weather', scene['weather'], '--weather-seed', str(scene['seed']),
                   '--weather-override', scene['overrides'], '--timeout', str(args.timeout)]
        print(f'Capturing {name}', flush=True)
        with (OUT / f'{name}.log').open('w') as log:
            try:
                result = subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT,
                                        timeout=args.timeout + 120)
                if result.returncode:
                    raise RuntimeError(f'exit {result.returncode}')
                data = json.loads(report.read_text())
                generation = data['generation']
                for key in ('missing_cells', 'in_flight_jobs', 'pending_tree_assets'):
                    if generation[key] != 0:
                        raise RuntimeError(f'{key}: {generation[key]}')
                providers = [provider for provider in data['network']['providers']
                             if provider['name'].startswith('async.')
                             and not provider['name'].endswith(':protomaps-build-resolver')]
                # Build discovery probes dates with no published archive before selecting
                # a compatible build. Reject actual scene-source failures, not those probes.
                if not providers or any(provider['errors'] or provider['in_flight']
                                        or provider['queued'] for provider in providers):
                    raise RuntimeError('scene data acquisition failed or remains pending')
                if data['render']['dropped_chunks'] or data['render_samples']['sun_shadow_capacity_drops']:
                    raise RuntimeError('renderer dropped geometry or shadow casters')
                if not data['render']['sun_shadow_configured']:
                    raise RuntimeError('sun shadows disabled')
                provenance = {'scene': scene, 'binary_sha256': binary_sha,
                              'image_sha256': hashlib.file_digest(image.open('rb'), 'sha256').hexdigest(),
                              'command': command, 'technical_checks': 'passed', 'visual_review': 'pending'}
                (OUT / f'{name}.provenance.json').write_text(json.dumps(provenance, indent=2)+'\n')
                print(f"Ready for review: {name}, {generation['wanted_cells']} cells, "
                      f"{data['timing']['stream_wall_ms']/1000:.1f}s", flush=True)
            except (RuntimeError, subprocess.TimeoutExpired, OSError, KeyError, ValueError) as error:
                failures.append(name)
                print(f'Rejected {name}: {error}', flush=True)
    if failures:
        raise SystemExit('Capture failures: ' + ', '.join(failures))


if __name__ == '__main__':
    main()
