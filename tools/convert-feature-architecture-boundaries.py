#!/usr/bin/env python3
"""Select complete municipal GeoJSON features from pinned source manifests.

Coordinates, properties, holes and detached components are preserved verbatim.
RFC 7946 GeoJSON uses WGS84; source URLs must explicitly request outSR=4326.
"""
import argparse
import hashlib
import json
from pathlib import Path
from urllib.parse import parse_qs, urlparse


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def run(manifest_path, check):
    manifest = json.loads(manifest_path.read_text())
    require(manifest['schema_version'] == 1, 'unknown manifest schema')
    directory = manifest_path.parent.resolve()

    def local(name):
        require(Path(name).name == name, 'asset must be a local basename')
        path = directory / name
        require(path.resolve().is_relative_to(directory), 'asset escapes source directory')
        return path

    if metadata := manifest.get('license_metadata'):
        require(digest(local(metadata['file']).read_bytes()) == metadata['sha256'],
                'license metadata checksum mismatch')
    for boundary in manifest['boundaries']:
        raw = local(boundary['source_snapshot']).read_bytes()
        require(digest(raw) == boundary['source_sha256'], 'source checksum mismatch')
        require(parse_qs(urlparse(boundary['source_url']).query).get('outSR') == ['4326'],
                'source must explicitly request WGS84')
        data = json.loads(raw)
        require(data.get('type') == 'FeatureCollection', 'expected FeatureCollection')
        require('crs' not in data, 'expected RFC 7946 GeoJSON without custom CRS')
        require(not data.get('exceededTransferLimit'), 'truncated source response')
        selector = boundary['selector']
        require(bool(selector), 'missing feature identity')
        matches = [f for f in data['features'] if all(
            f.get('properties', {}).get(k) == v for k, v in selector.items())]
        require(len(matches) == 1, 'feature identity must match exactly once')
        feature = matches[0]
        geometry = feature['geometry']
        require(feature.get('type') == 'Feature' and 'crs' not in feature,
                'expected a complete unambiguous Feature')
        require(geometry.get('type') in ('Polygon', 'MultiPolygon') and 'crs' not in geometry,
                'expected WGS84 polygon geometry')
        components = ([geometry['coordinates']] if geometry['type'] == 'Polygon'
                      else geometry['coordinates'])
        require(len(components) == boundary['components']
                and sum(map(len, components)) == boundary['rings']
                and sum(len(r) for c in components for r in c) == boundary['coordinate_pairs'],
                'geometry structure changed')
        output = (json.dumps(feature, ensure_ascii=False, indent=2) + '\n').encode()
        require(digest(output) == boundary['sha256'], 'derived checksum mismatch')
        path = local(boundary['file'])
        if check:
            require(path.read_bytes() == output, f'stale derived feature: {path}')
        else:
            path.write_bytes(output)
        print(f"{boundary['label']}: {'verified' if check else 'rebuilt'} complete feature")


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifests', type=Path, nargs='+')
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    for manifest_path in args.manifests:
        run(manifest_path, args.check)
