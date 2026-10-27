#!/usr/bin/env python3
"""Topology and provenance checks for the pinned OSM boundary converter."""

import importlib.util
import json
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('converter', Path(__file__).with_name('convert-osm-architecture-boundaries.py'))
converter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(converter)


class BoundaryTests(unittest.TestCase):
    def test_pinned_boundaries_reproduce_exact_bytes(self):
        converter.run(check=True)

    def test_reversed_fragments_and_multiple_components(self):
        self.assertEqual(converter.join_rings([[1, 2, 3], [1, 4, 3], [5, 6, 7, 5]]),
                         [[1, 2, 3, 4, 1], [5, 6, 7, 5]])

    def test_open_and_ambiguous_chains_are_rejected(self):
        for ways in ([[1, 2], [3, 4]], [[1, 2], [2, 3, 1], [2, 4, 1]]):
            with self.assertRaises(ValueError):
                converter.join_rings(ways)

    def test_dateline_hole_and_separate_component_are_preserved(self):
        rings = [
            [(179.9, 0), (-179.9, 0), (-179.9, 0.1), (179.9, 0.1)],
            [(179.95, 0.02), (-179.95, 0.02), (-179.95, 0.08), (179.95, 0.08)],
            [(179.9, 0.2), (-179.9, 0.2), (-179.9, 0.3), (179.9, 0.3)],
        ]
        nodes, ways, members = [], [], []
        for index, ring in enumerate(rings):
            ids = []
            for lon, lat in ring:
                node_id = len(nodes) + 1
                ids.append(node_id)
                nodes.append(dict(type='node', id=node_id, lon=lon, lat=lat))
            ways.append(dict(type='way', id=index + 1, nodes=ids + ids[:1]))
            members.append(dict(type='way', ref=index + 1, role='inner' if index == 1 else 'outer'))
        relation = dict(type='relation', id=1, version=1, members=members,
                        tags=dict(type='boundary', name='test', wikidata='Q1'))
        raw = json.dumps(dict(elements=nodes + ways + [relation])).encode()
        expected = dict(relation_id=1, relation_version=1, wikidata='Q1',
                        components=2, rings=3, coordinate_pairs=15, scope='test')
        result = json.loads(converter.convert(raw, expected))['geometry']
        self.assertEqual(result['type'], 'MultiPolygon')
        self.assertEqual([len(polygon) for polygon in result['coordinates']], [2, 1])
        self.assertEqual(result['coordinates'][0][1], [list(p) for p in rings[1] + rings[1][:1]])
        self.assertEqual(result['coordinates'][1][0], [list(p) for p in rings[2] + rings[2][:1]])

    def test_adjacent_district_union_removes_only_shared_way(self):
        nodes = [dict(type='node', id=i + 1, lon=x, lat=y)
                 for i, (x, y) in enumerate([(0, 0), (1, 0), (1, 1), (0, 1), (2, 0), (2, 1)])]
        ways = [dict(type='way', id=i + 1, nodes=pair)
                for i, pair in enumerate([[1, 2], [2, 3], [3, 4], [4, 1], [2, 5], [5, 6], [6, 3]])]
        raws, sources = [], []
        for i, refs in enumerate([[1, 2, 3, 4], [5, 6, 7, 2]], 1):
            relation = dict(type='relation', id=i, version=1,
                            tags=dict(type='boundary', wikidata=f'Q{i}', name=f'district {i}'),
                            members=[dict(type='way', ref=ref, role='outer') for ref in refs])
            raws.append(json.dumps(dict(elements=nodes + ways + [relation])).encode())
            sources.append(dict(relation_id=i, relation_version=1, wikidata=f'Q{i}'))
        expected = dict(label='union', sources=sources, components=1, rings=1,
                        coordinate_pairs=7, scope='adjacent districts')
        feature = json.loads(converter.convert_union(raws, expected))
        ring = feature['geometry']['coordinates'][0]
        self.assertEqual(ring, [[0, 0], [1, 0], [2, 0], [2, 1], [1, 1], [0, 1], [0, 0]])
        self.assertTrue(converter.inside((0.5, 0.5), ring))
        self.assertTrue(converter.inside((1.5, 0.5), ring))
        self.assertFalse(converter.inside((2.5, 0.5), ring))
        changed = json.loads(raws[1])
        changed['elements'][0]['lon'] = 0.01
        with self.assertRaisesRegex(ValueError, 'inconsistent shared source element'):
            converter.convert_union([raws[0], json.dumps(changed).encode()], expected)


if __name__ == '__main__':
    unittest.main()
