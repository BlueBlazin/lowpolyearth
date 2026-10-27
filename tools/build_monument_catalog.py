#!/usr/bin/env python3
"""Bake the thirty additional monuments; --check verifies assets and provenance.

Requires Python and NumPy. --only accepts site or group names during authoring.
Foundations follow frozen raw terrain; the surrounding heightfield is unchanged.
"""
import argparse
import importlib
import json
from pathlib import Path
import re
import struct
import zlib

import numpy as np

from build_city_catalog import digest, encode
from build_heritage_catalog import registry
from heritage_catalog.materials import validate_runtime_ids
from monument_catalog.inventory import GROUPS, NAMES
from monument_catalog.grounding import Survey, components, fit, support_shifts

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / 'tools/monument_catalog'
OUTPUT = ROOT / 'crates/vearth-gen/assets/landmarks/monument-surfaces'
REGISTRY = ROOT / 'crates/vearth-gen/src/landmarks/monument_generated.rs'


def bake(module, entry):
    name = entry['name']
    assert entry.get('references'), f'{name}: architectural references required'
    assert np.isfinite([entry['lat'], entry['lon'], entry['heading']]).all(), name
    assert abs(entry['lat']) <= 85 and abs(entry['lon']) <= 180, name
    assert 0 <= entry['heading'] < 360, name
    assert not any(entry.get(k) for k in ('terrain_flatten', 'terrain_cuts', 'water_cuts', 'terrain_grades')), name
    survey = Survey(entry)
    shifts = support_shifts(components(module, name, 0), survey, entry)
    entry = {**entry, 'base': survey.base,
             'grounding': {'component_shifts_m': shifts, 'toe_burial_m': .5,
                           'terrain_modified': False}}
    if 'passage_probe' in entry:
        entry['passage_probe'] = {**entry['passage_probe'],
                                  'y': entry['passage_probe']['y']+max(shifts)}
    levels, stats, points = [], [], []
    for lod in range(3):
        data, stat, p = encode(fit(components(module, name, lod), shifts, survey))
        assert 0 < stat['triangles'] <= 500_000, (name, lod, stat)
        assert not stats or stat['triangles'] <= stats[-1]['triangles'], (name, 'LOD grows')
        levels.append(data)
        stats.append(stat)
        points.append(p)
    raw = b'LMS1' + struct.pack('<I', 3) + b''.join(levels)
    assert len(raw) <= 256 * 1024 * 1024, (name, 'decode budget')
    payload = struct.pack('<I', len(raw)) + zlib.compress(raw, 9)
    points = np.concatenate(points).astype(np.float64)
    low = np.round(np.floor(points.min(axis=0)*10)/10-.2, 1)
    high = np.round(np.ceil(points.max(axis=0)*10)/10+.2, 1)
    suppression = np.asarray(entry['suppress_bbox'], float)
    assert suppression.shape == (2, 2) and np.isfinite(suppression).all(), name
    assert (suppression[1] > suppression[0]).all(), name
    low[[0, 2]] = np.minimum(low[[0, 2]], suppression[0])
    high[[0, 2]] = np.maximum(high[[0, 2]], suppression[1])
    dependencies = [Path(__file__), SOURCE/'grounding.py', survey.path,
                    ROOT/'tools/survey_monument_ground.py', Path(module.__file__), SOURCE/'inventory.py',
                    ROOT/'tools/build_city_catalog.py', ROOT/'tools/build_heritage_catalog.py',
                    ROOT/'tools/city_catalog/common.py', ROOT/'tools/city_catalog/expansion_heritage.py',
                    ROOT/'tools/heritage_catalog/geometry.py', ROOT/'tools/heritage_catalog/materials.py',
                    *getattr(module, 'DEPENDENCIES', [])]
    dependencies.extend(p for p in (SOURCE/'references'/module.__name__.split('.')[-1]).rglob('*')
                        if p.is_file() and p.suffix not in ('.pyc', '.pyo'))
    record = {**entry, 'bbox_min': low.tolist(), 'bbox_max': high.tolist(), 'levels': stats,
              'suppress_bbox': suppression.tolist(),
              'geometry': 'original metric interpretation of cited references; estimates identified in source notes',
              'license': 'repository LICENSE; no reference images or external meshes embedded',
              'source': str(Path(module.__file__).relative_to(ROOT)),
              'dependencies': {str(p.relative_to(ROOT)): digest(p.read_bytes())
                               for p in sorted(set(map(Path, dependencies)))},
              'output_sha256': digest(payload)}
    return payload, record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    parser.add_argument('--only', nargs='+', choices=(*NAMES, *GROUPS))
    args = parser.parse_args()
    validate_runtime_ids()
    existing = set()
    for path in (ROOT/'crates/vearth-gen/src/landmarks').glob('*.rs'):
        if path != REGISTRY:
            existing.update(re.findall(r'name: "([^"]+)"', path.read_text()))
    assert not existing.intersection(NAMES), 'duplicate existing monument'
    assert len(NAMES) == len(set(NAMES)) == 30, 'pack must contain exactly 30 sites'
    selected = set(NAMES) if not args.only else {n for v in args.only for n in GROUPS.get(v, (v,))}
    outputs, records = {}, {}
    for group, names in GROUPS.items():
        if not selected.intersection(names):
            continue
        module = importlib.import_module('monument_catalog.'+group)
        entries = {e['name']: e for e in module.ENTRIES}
        assert len(entries) == len(module.ENTRIES) and set(entries) == set(names), group
        for name in names:
            if name not in selected:
                continue
            payload, record = bake(module, entries[name])
            outputs[OUTPUT/(name+'.lms')] = payload
            outputs[OUTPUT/(name+'.source.json')] = (json.dumps(record, indent=2)+'\n').encode()
            records[name] = record
            print(name, record['levels'], flush=True)
    for name in NAMES:
        if name not in records:
            p = OUTPUT/(name+'.source.json')
            if p.exists():
                record = json.loads(p.read_text())
                assert digest((OUTPUT/(name+'.lms')).read_bytes()) == record['output_sha256'], name
                records[name] = record
    entries = [records[n] for n in NAMES if n in records]
    if not args.only:
        assert len(entries) == 30
    code = registry(entries)
    for old, new in [(b'build_heritage_catalog.py', b'build_monument_catalog.py'),
                     (b'HERITAGE_MODELS', b'MONUMENT_MODELS'),
                     (b'/heritage-surfaces/', b'/monument-surfaces/')]:
        assert old in code
        code = code.replace(old, new)
    outputs[REGISTRY] = code
    outputs[OUTPUT/'catalog.json'] = (json.dumps(entries, indent=2)+'\n').encode()
    for path, data in outputs.items():
        if args.check:
            assert path.exists() and path.read_bytes() == data, f'stale generated monument: {path}'
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)


if __name__ == '__main__':
    main()
