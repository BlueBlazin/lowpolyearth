#!/usr/bin/env python3
"""Verify pinned Westminster boundaries, or rebuild them with pyproj.

Normal atlas builds consume the derived GeoJSON. This optional conversion uses
an explicit PROJ pipeline so installed datum grids cannot change the operation.
"""

import argparse
import csv
import hashlib
import io
import json
import math
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / 'crates/vearth-gen/assets/architecture/westminster'
MANIFEST = ASSETS / 'boundaries.source.json'


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def source_polygons(wkt):
    kind, body = wkt.split(' ', 1)
    require(kind == 'MULTIPOLYGON', 'expected pinned XY MULTIPOLYGON')
    require(re.fullmatch(r'[()\s,0-9.+-]+', body), 'unsupported WKT ordinates')
    # The hashed source contains XY pairs only. Preserve every component, ring,
    # closing coordinate and vertex order rather than rebuilding an outline.
    body = re.sub(r'([-+\d.]+) ([-+\d.]+)', r'[\1,\2]', body)
    return json.loads(body.replace('(', '[').replace(')', ']'))


def validate_coordinates(polygons, expected, geographic=False):
    require(len(polygons) == expected['components'], 'component count changed')
    require(sum(map(len, polygons)) == expected['rings'], 'ring count changed')
    require(sum(len(ring) for polygon in polygons for ring in polygon)
            == expected['coordinate_pairs'], 'coordinate count changed')
    for polygon in polygons:
        require(bool(polygon), 'empty polygon')
        for ring in polygon:
            require(len(ring) >= 4 and ring[0] == ring[-1], 'invalid ring closure')
            for point in ring:
                require(len(point) == 2 and all(math.isfinite(v) for v in point),
                        'invalid coordinate pair')
                if geographic:
                    require(abs(point[0]) <= 180 and abs(point[1]) <= 90,
                            'coordinate outside WGS84 bounds')


def pinned_file(name):
    require(Path(name).name == name, 'manifest filenames must be local basenames')
    return ASSETS / name


def run(check=False):
    manifest = json.loads(MANIFEST.read_text())
    require(manifest['schema_version'] == 1, 'unknown boundary manifest schema')
    source = manifest['source']
    raw = pinned_file(source['file']).read_bytes()
    require(digest(raw) == source['sha256'], 'pinned council CSV checksum mismatch')
    rows = list(csv.DictReader(io.StringIO(raw.decode('utf-8-sig'))))
    transformer = None
    if not check:
        try:
            from pyproj import Transformer
        except ImportError as error:
            raise SystemExit('Rebuilding requires pyproj; --check uses standard Python only.') from error
        transformer = Transformer.from_pipeline(manifest['conversion']['pipeline'])
    for boundary in manifest['boundaries']:
        matches = [row for row in rows if row['Reference'] == boundary['reference']]
        require(len(matches) == 1, 'boundary reference must select exactly one row')
        row = matches[0]
        require(row['Name'] == boundary['name']
                and row['Organisation'] == source['organisation'], 'source identity changed')
        polygons = source_polygons(row[source['geometry_field']])
        validate_coordinates(polygons, boundary)
        target = pinned_file(boundary['file'])
        if check:
            data = target.read_bytes()
        else:
            coordinates = [[[list(transformer.transform(*point, errcheck=True))
                             for point in ring] for ring in polygon] for polygon in polygons]
            feature = {
                'type': 'Feature',
                'properties': {key: value for key, value in row.items()
                               if key != source['geometry_field']},
                'geometry': {'type': 'MultiPolygon', 'coordinates': coordinates},
            }
            data = (json.dumps(feature, indent=2) + '\n').encode()
        require(digest(data) == boundary['sha256'],
                'derived boundary checksum mismatch; review conversion before changing provenance')
        feature = json.loads(data)
        require(feature['type'] == 'Feature'
                and feature['geometry']['type'] == 'MultiPolygon', 'invalid derived geometry')
        require(feature['properties'] == {key: value for key, value in row.items()
                                         if key != source['geometry_field']},
                'derived source properties changed')
        validate_coordinates(feature['geometry']['coordinates'], boundary, geographic=True)
        if not check:
            target.write_bytes(data)
        print(f"{boundary['name']}: {boundary['coordinate_pairs']} coordinate pairs; "
              f"{'verified' if check else 'rebuilt'} {boundary['sha256']}")


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', help='verify pinned hashes without pyproj or writes')
    run(parser.parse_args().check)
