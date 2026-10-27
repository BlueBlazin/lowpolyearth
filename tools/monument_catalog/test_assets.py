"""Consumer-level checks of baked monument colors, openings, and provenance."""
import hashlib
import importlib
import json
from pathlib import Path
import struct
import unittest
import zlib

import numpy as np

from build_city_catalog import VERTEX
from monument_catalog.inventory import GROUPS, NAMES
from monument_catalog.grounding import Survey, components, fit, support_shifts

ROOT = Path(__file__).resolve().parents[2]
ASSETS = ROOT/'crates/vearth-gen/assets/landmarks/monument-surfaces'


def decode(path):
    payload = path.read_bytes()
    raw = zlib.decompress(payload[4:])
    assert len(raw) == struct.unpack_from('<I', payload)[0]
    assert raw[:4] == b'LMS1' and struct.unpack_from('<I', raw, 4)[0] == 3
    offset = 8
    levels = []
    for _ in range(3):
        nv, ni = struct.unpack_from('<II', raw, offset)
        offset += 8
        vertices = np.frombuffer(raw, dtype=VERTEX, count=nv, offset=offset)
        offset += nv*VERTEX.itemsize
        indices = np.frombuffer(raw, dtype='<u4', count=ni, offset=offset).reshape(-1, 3)
        offset += ni*4
        assert indices.max() < nv
        levels.append((vertices, indices))
    assert offset == len(raw)
    return levels


def passage_intersections(triangles, probe):
    start, end = probe['z_range']
    origin = np.asarray([probe['x'], probe['y'], start], float)
    direction = np.asarray([0, 0, end-start], float)
    edge1 = triangles[:, 1]-triangles[:, 0]
    edge2 = triangles[:, 2]-triangles[:, 0]
    p = np.cross(direction, edge2)
    det = np.einsum('ij,ij->i', edge1, p)
    candidates = np.abs(det) > 1e-8
    inv = np.zeros_like(det)
    inv[candidates] = 1/det[candidates]
    tvec = origin-triangles[:, 0]
    u = np.einsum('ij,ij->i', tvec, p)*inv
    q = np.cross(tvec, edge1)
    v = q@direction*inv
    t = np.einsum('ij,ij->i', edge2, q)*inv
    return candidates & (u >= -1e-7) & (v >= -1e-7) & (u+v <= 1+1e-7) & (t > 0) & (t < 1)


class MonumentAssets(unittest.TestCase):
    def test_all_thirty_preserve_terrain_and_ground_existing_feet(self):
        for group in GROUPS:
            module = importlib.import_module('monument_catalog.'+group)
            for authored in module.ENTRIES:
                name = authored['name']
                entry = json.loads((ASSETS/(name+'.source.json')).read_text())
                with self.subTest(name=name):
                    for key in ('terrain_flatten', 'terrain_cuts', 'terrain_grades', 'water_cuts'):
                        self.assertFalse(entry.get(key), (name, key))
                    survey = Survey(authored)
                    self.assertEqual(entry['base'], survey.base)
                    shifts = support_shifts(components(module, name, 0), survey, authored)
                    self.assertEqual(entry['grounding']['component_shifts_m'], shifts)
                    for lod in range(3):
                        parts = components(module, name, lod)
                        fitted = fit(parts, shifts, survey)
                        offset = 0
                        for part, shift in zip(parts, shifts):
                            count = len(part.triangles)
                            points = np.concatenate([t[0] for t in part.triangles])
                            actual = np.concatenate([t[0] for t in fitted.triangles[offset:offset+count]])
                            self.assertEqual([(t[1], t[2]) for t in part.triangles],
                                [(t[1], t[2]) for t in fitted.triangles[offset:offset+count]])
                            offset += count
                            np.testing.assert_array_equal(actual[:, [0, 2]], points[:, [0, 2]])
                            upper = points[:, 1] >= -.001
                            np.testing.assert_allclose(actual[upper, 1], points[upper, 1]+shift, atol=1e-9)
                            toes = ~upper
                            self.assertTrue((actual[toes, 1]+survey.base <=
                                survey.height(points[toes][:, [0, 2]])-.499).all())
                    # Separated stones must follow the sampled slope instead
                    # of inheriting one flattened site-wide placement height.
                    if name in ('stonehenge', 'callanish-stones', 'ring-of-brodgar'):
                        self.assertGreater(np.ptp(shifts), .1)

    def test_all_assets_are_current_and_keep_detailed_color_at_every_lod(self):
        catalog = json.loads((ASSETS/'catalog.json').read_text())
        self.assertEqual([e['name'] for e in catalog], list(NAMES))
        self.assertEqual(len(catalog), 30)
        checked_dependencies = set()
        for entry in catalog:
            name = entry['name']
            with self.subTest(name=name):
                self.assertEqual(entry, json.loads((ASSETS/(name+'.source.json')).read_text()))
                path = ASSETS/(name+'.lms')
                self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), entry['output_sha256'])
                for dependency, expected in entry['dependencies'].items():
                    if (dependency, expected) in checked_dependencies:
                        continue
                    self.assertEqual(hashlib.sha256((ROOT/dependency).read_bytes()).hexdigest(), expected, dependency)
                    checked_dependencies.add((dependency, expected))
                for lod, (vertices, indices) in enumerate(decode(path)):
                    self.assertEqual(len(indices), entry['levels'][lod]['triangles'])
                    self.assertEqual(len(vertices), entry['levels'][lod]['vertices'])
                    colors = vertices['color']
                    # Distant LODs omit fine relief but retain the principal
                    # body/trim distinction rather than becoming monochrome.
                    self.assertGreaterEqual(len(np.unique(colors[:, :3], axis=0)), 3 if lod == 0 else 2,
                                            f'{name} LOD {lod} loses stonework/material tint')
                    self.assertTrue(((colors[:, 3] > 0) & (colors[:, 3] < 128)).all())
                    self.assertTrue(np.isfinite(vertices['position']).all())

    def test_gateway_collision_keeps_passages_open_at_every_lod(self):
        for name in GROUPS['japanese']+GROUPS['gateways']:
            entry = json.loads((ASSETS/(name+'.source.json')).read_text())
            self.assertIn('passage_probe', entry, name)
            probe = entry['passage_probe']
            for lod, (vertices, indices) in enumerate(decode(ASSETS/(name+'.lms'))):
                with self.subTest(name=name, lod=lod):
                    triangles = vertices['position'][indices].astype(float)
                    # Both a head-height and slightly offset ray should pass;
                    # testing off-axis also catches a coincident seam hole.
                    for dx in (0, .12, -.12):
                        moved = {**probe, 'x': probe['x']+dx}
                        self.assertFalse(passage_intersections(triangles, moved).any(),
                                         f'{name} LOD {lod}: blocked gateway')


if __name__ == '__main__':
    unittest.main()
