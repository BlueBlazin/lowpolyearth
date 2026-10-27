#!/usr/bin/env python3
"""Verify authored district vertices against lossless pinned OSM snapshots."""

import argparse
import gzip
import hashlib
import json
from pathlib import Path
import xml.etree.ElementTree as ET


ASSETS = Path(__file__).resolve().parents[1] / 'crates/vearth-gen/assets/architecture/authored-districts'


def require(condition, message):
    if not condition:
        raise ValueError(message)


def checked_bytes(record):
    relative = Path(record['path'])
    require(not relative.is_absolute() and '..' not in relative.parts, 'invalid snapshot path')
    path = (ASSETS / relative).resolve()
    require(path.is_relative_to(ASSETS.resolve()), 'snapshot escapes asset directory')
    data = path.read_bytes()
    require(hashlib.sha256(data).hexdigest() == record['sha256'], f'hash mismatch: {relative}')
    return data


def source_nodes(record):
    raw = gzip.decompress(checked_bytes(record))
    require(hashlib.sha256(raw).hexdigest() == record['uncompressed_sha256'],
            f'uncompressed hash mismatch: {record["path"]}')
    nodes, ways = {}, {}
    if record['format'] == 'osm-xml':
        root = ET.fromstring(raw)
        require(root.tag == 'osm', 'expected OSM XML snapshot')
        for node in root.findall('node'):
            nodes[int(node.attrib['id'])] = [float(node.attrib['lon']), float(node.attrib['lat'])]
        for way in root.findall('way'):
            ways[int(way.attrib['id'])] = {int(node.attrib['ref']) for node in way.findall('nd')}
    else:
        require(record['format'] == 'overpass-json', 'unknown source format')
        for element in json.loads(raw)['elements']:
            if element['type'] == 'node':
                nodes[element['id']] = [element['lon'], element['lat']]
            elif element['type'] == 'way':
                ids, geometry = element['nodes'], element['geometry']
                require(len(ids) == len(geometry), 'incomplete Overpass geometry')
                ways[element['id']] = set(ids)
                for node_id, point in zip(ids, geometry):
                    nodes[node_id] = [point['lon'], point['lat']]
    return nodes, ways


def run():
    manifest = json.loads((ASSETS / 'districts.source.json').read_text())
    require(manifest['schema_version'] == 1, 'unsupported manifest version')
    sources = [source_nodes(record) for record in manifest['sources']]
    anchors = json.loads(checked_bytes(manifest['anchors']))
    districts = {record['district'] for record in manifest['polygons']}
    require({anchor['district'] for anchor in anchors} == districts, 'unmatched district anchors')
    for anchor in anchors:
        require(any(nodes.get(anchor['node_id']) == anchor['coordinate']
                    and ('way_id' not in anchor or
                         anchor['node_id'] in ways.get(anchor['way_id'], set()))
                    for nodes, ways in sources), f'unverified OSM anchor: {anchor}')
    for record in manifest['polygons']:
        feature = json.loads(checked_bytes(record))
        require(feature['type'] == 'Feature', 'expected one authored GeoJSON feature')
        geometry = feature['geometry']
        selected = [a['coordinate'] for a in anchors if a['district'] == record['district']]
        require(len(selected) >= 3 and len({tuple(p) for p in selected}) == len(selected),
                'polygon needs distinct source vertices')
        require(geometry == {'type': 'Polygon', 'coordinates': [selected + selected[:1]]},
                f'polygon differs from ordered source anchors: {record["district"]}')
    print(f'Verified {len(districts)} authored district polygons and {len(anchors)} OSM anchors.')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', help='verify all snapshots and polygon vertices')
    parser.parse_args()
    run()
