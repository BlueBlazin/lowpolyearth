#!/usr/bin/env python3
"""Validate all fortress LODs with the production encoder and convex ownership."""
import hashlib
import json
from pathlib import Path
import sys

import numpy as np

REF = Path(__file__).resolve().parent
sys.path.insert(0, str(REF.parents[2]))
from build_city_catalog import encode
from heritage_catalog.fortresses import BUILDERS, ENTRIES, build, plan, datum, Mesh, face


def main():
    strip=Mesh();face(strip,[(0,0,0),(0,0,0),(2,0,1),(0,0,1)],(100,100,100))
    assert len(strip.triangles)==1
    p=strip.triangles[0][0];assert np.cross(p[1]-p[0],p[2]-p[0])[1]>0
    source = REF.parents[1] / 'fortresses.py'
    report = {'source_sha256': hashlib.sha256(source.read_bytes()).hexdigest(), 'levels': {}, 'metadata': {}}
    for entry in ENTRIES:
        name = entry['name']
        site = BUILDERS[name][0]
        assert entry['base'] == datum(site)
        assert entry['heading'] == 0 and not entry['terrain_flatten']
        terrain = json.loads((REF / (site + '-terrain.json')).read_text())
        samples = terrain['ground_samples']
        assert samples and all(np.isfinite(s['ground_m']) for s in samples)
        assert len(plan(site)['ways']) > 0
        for polygon in entry['source_exclusion_polygons']:
            points = np.asarray(polygon, dtype=float)
            assert points.shape[1] == 2 and len(points) in (3, 4) and np.isfinite(points).all()
            edges = np.roll(points, -1, axis=0) - points
            next_edges = np.roll(edges, -1, axis=0)
            cross = edges[:, 0]*next_edges[:, 1] - edges[:, 1]*next_edges[:, 0]
            assert np.all(cross >= -1e-6) or np.all(cross <= 1e-6), (name, polygon)
            assert np.max(abs(cross)) > 1e-8
        for cut in entry['terrain_cuts']:
            assert np.isfinite(cut['polygon_xz']).all() and np.isfinite(cut['plane']).all()
            assert len(cut['polygon_xz'])==3 and cut['feather_m']>0
        report['levels'][name] = []
        for lod in range(3):
            payload, counts, positions = encode(build(name, lod))
            report['levels'][name].append({**counts, 'minimum': positions.min(axis=0).tolist(),
                                          'maximum': positions.max(axis=0).tolist(),
                                          'encoded_sha256': hashlib.sha256(payload).hexdigest()})
        counts = [v['triangles'] for v in report['levels'][name]]
        assert counts[0] >= counts[1] >= counts[2]
        report['metadata'][name] = {'datum_m': entry['base'], 'ground_samples': len(samples),
                                   'source_polygons': len(entry['source_exclusion_polygons']),
                                   'terrain_sha256': hashlib.sha256((REF/(site+'-terrain.json')).read_bytes()).hexdigest(),
                                   'plan_sha256': hashlib.sha256((REF/(site+'-plan.json')).read_bytes()).hexdigest()}
        print(name, counts, flush=True)
    (REF/'mesh-validation.json').write_text(json.dumps(report, indent=2)+'\n')


if __name__ == '__main__':
    main()
