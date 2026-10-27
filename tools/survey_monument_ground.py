#!/usr/bin/env python3
"""Freeze unmodified production terrain in model coordinates for monument feet."""
import concurrent.futures
import importlib
import json
import math
import os
from pathlib import Path
import subprocess
import tempfile

import numpy as np
import build_city_catalog  # Establish shared geometry helper import paths.
from monument_catalog.inventory import GROUPS

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT/'tools/monument_catalog/references/ground'


def survey(job):
    module_name, entry = job
    module = importlib.import_module('monument_catalog.'+module_name)
    points = np.concatenate([t[0] for t in module.build(entry['name'], 0).triangles])
    xs = np.arange(math.floor(points[:, 0].min())-2, math.ceil(points[:, 0].max())+3, 2.)
    zs = np.arange(math.floor(points[:, 2].min())-2, math.ceil(points[:, 2].max())+3, 2.)
    h = math.radians(entry['heading'])
    offsets = [[x*math.cos(h)+z*math.sin(h), -x*math.sin(h)+z*math.cos(h)]
               for z in zs for x in xs]
    assert len(offsets) <= 16641
    with tempfile.TemporaryDirectory(prefix='monument-ground-') as tmp:
        query, result = Path(tmp)/'points.json', Path(tmp)/'result.json'
        query.write_text(json.dumps(offsets))
        subprocess.run([str(ROOT/'target/debug/city-survey'), str(entry['lat']), str(entry['lon']),
                        str(result), '--ground-only', '--ground-points', str(query)],
                       cwd=ROOT, env={**os.environ, 'VEARTH_ASYNC_PROVIDERS': '0'},
                       check=True, capture_output=True, timeout=180)
        raw = json.loads(result.read_text())
    record = {'name': entry['name'], 'lat': entry['lat'], 'lon': entry['lon'],
              'heading': entry['heading'], 'ground_m': raw['ground_m'],
              'x_m': xs.tolist(), 'z_m': zs.tolist(),
              'heights_m': np.array([s['ground_m'] for s in raw['ground_samples']]).reshape(len(zs), len(xs)).tolist(),
              'source': 'city-survey production terrain; 8 m source sampling pitch; no landmark correction',
              'acquisition': 'python3 tools/survey_monument_ground.py'}
    OUTPUT.mkdir(parents=True, exist_ok=True)
    (OUTPUT/(entry['name']+'.json')).write_text(json.dumps(record, indent=2)+'\n')
    print(entry['name'], len(offsets), 'samples', flush=True)


if __name__ == '__main__':
    jobs = [(group, e) for group in GROUPS for e in importlib.import_module('monument_catalog.'+group).ENTRIES]
    with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
        list(pool.map(survey, jobs))
