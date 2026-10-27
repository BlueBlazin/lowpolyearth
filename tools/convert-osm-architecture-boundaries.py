#!/usr/bin/env python3
"""Rebuild or verify pinned local OSM boundary relations using standard Python.

Join ways by exact node identity, retaining all source coordinates. Unsupported
relation members and ambiguous chains fail instead of dropping geometry.
"""

import argparse
import hashlib
import json
import math
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / 'crates/vearth-gen/assets/architecture/osm'
MANIFEST = ASSETS / 'boundaries.source.json'


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def pinned_file(name, directory=ASSETS):
    require(isinstance(name, str) and Path(name).name == name,
            'source and output filenames must be local basenames')
    path = directory / name
    require(path.resolve().is_relative_to(directory.resolve()), 'asset path escapes directory')
    return path


def join_rings(ways):
    """Preserve source member order and exact shared-node topology."""
    pending = [way[:] for way in ways]
    rings = []
    while pending:
        ring = pending.pop(0)
        require(len(ring) >= 2, 'source way needs at least two nodes')
        while ring[0] != ring[-1]:
            candidates = [(index, way if way[0] == ring[-1] else way[::-1])
                          for index, way in enumerate(pending)
                          if way[0] == ring[-1] or way[-1] == ring[-1]]
            require(len(candidates) == 1, 'open or ambiguous boundary chain')
            index, way = candidates[0]
            ring.extend(way[1:])
            pending.pop(index)
        require(len(ring) >= 4 and len(set(ring[:-1])) == len(ring) - 1,
                'degenerate or self-touching boundary ring')
        rings.append(ring)
    return rings


def inside(point, ring):
    # Local envelopes may cross the dateline. Center longitudes on the query
    # before planar containment; the runtime separately validates local caps.
    x, y = point
    points = [(((lon - x + 180) % 360) - 180, lat) for lon, lat in ring]
    result = False
    for (ax, ay), (bx, by) in zip(points, points[1:]):
        if (ay > y) != (by > y) and 0 < ax + (bx - ax) * (y - ay) / (by - ay):
            result = not result
    return result


def convert(raw, expected):
    elements = json.loads(raw)['elements']
    index = {(item['type'], item['id']): item for item in elements}
    require(len(index) == len(elements), 'duplicate source element identity')
    relation = index['relation', expected['relation_id']]
    require(relation['version'] == expected['relation_version'], 'source relation version changed')
    require(relation['tags'].get('wikidata') == expected['wikidata'], 'source identity changed')
    require(relation['tags'].get('type') in ('boundary', 'multipolygon'), 'not a boundary relation')
    members = relation['members']
    require(all((m['type'] == 'way' and m['role'] in ('outer', 'inner'))
                or (m['type'] == 'node' and m['role'] in ('label', 'admin_centre'))
                for m in members), 'unsupported relation member; cannot omit geometry')
    require(len({(m['type'], m['ref']) for m in members}) == len(members), 'duplicate relation member')

    def coordinates(role):
        rings = join_rings([index['way', m['ref']]['nodes'] for m in members if m['role'] == role])
        result = []
        for ring in rings:
            points = [[index['node', node]['lon'], index['node', node]['lat']] for node in ring]
            require(all(math.isfinite(lon) and math.isfinite(lat) and abs(lon) <= 180 and abs(lat) <= 90
                        for lon, lat in points), 'invalid WGS84 source coordinate')
            result.append(points)
        return result

    outers, inners = coordinates('outer'), coordinates('inner')
    require(bool(outers), 'boundary has no exterior')
    polygons = [[ring] for ring in outers]
    for hole in inners:
        owners = [polygon for polygon in polygons if inside(hole[0], polygon[0])]
        require(len(owners) == 1, 'hole must belong to exactly one exterior')
        require(all(inside(point, owners[0][0]) for point in hole[:-1]), 'hole crosses exterior')
        owners[0].append(hole)
    require(len(polygons) == expected['components'], 'component count changed')
    require(sum(map(len, polygons)) == expected['rings'], 'ring count changed')
    require(sum(len(ring) for polygon in polygons for ring in polygon) == expected['coordinate_pairs'],
            'coordinate count changed')
    geometry = ({'type': 'Polygon', 'coordinates': polygons[0]} if len(polygons) == 1
                else {'type': 'MultiPolygon', 'coordinates': polygons})
    feature = {
        'type': 'Feature',
        'properties': {
            'name': relation['tags']['name'],
            'osm_relation': relation['id'],
            'osm_version': relation['version'],
            'source_url': f"https://www.openstreetmap.org/relation/{relation['id']}",
            'source_snapshot_sha256': digest(raw),
            'license': 'ODbL-1.0; © OpenStreetMap contributors',
            'scope': expected['scope'],
            'conversion': 'Closed member ways joined by exact shared node identities; no resampling, simplification or coordinate snapping.',
        },
        'geometry': geometry,
    }
    return (json.dumps(feature, ensure_ascii=False, indent=2) + '\n').encode()


def convert_union(raws, expected):
    """Union adjacent, hole-free districts by cancelling identical shared ways.

    This operation intentionally rejects general polygon overlays: it never
    intersects, rounds or snaps source coordinates. The source relation group
    must describe adjacent mapped districts with shared OSM way identities.
    """
    require(len(raws) == len(expected['sources']) >= 2, 'union needs source relations')
    index, members, counts = {}, {}, {}
    for raw, source in zip(raws, expected['sources']):
        elements = json.loads(raw)['elements']
        local = {(item['type'], item['id']): item for item in elements}
        require(len(local) == len(elements), 'duplicate source element identity')
        relation = local['relation', source['relation_id']]
        require(relation['version'] == source['relation_version'], 'source relation version changed')
        require(relation['tags'].get('wikidata') == source['wikidata'], 'source identity changed')
        require(relation['tags'].get('type') in ('boundary', 'multipolygon'), 'not a boundary relation')
        boundary = relation['members']
        require(all(m['type'] == 'way' and m['role'] == 'outer' for m in boundary),
                'union supports only hole-free way-only district relations')
        require(len({m['ref'] for m in boundary}) == len(boundary), 'duplicate relation member')
        for key, element in local.items():
            require(key not in index or index[key] == element, 'inconsistent shared source element')
            index[key] = element
        for member in boundary:
            ref = member['ref']
            members[ref] = member
            counts[ref] = counts.get(ref, 0) + 1
            require(counts[ref] <= 2, 'a shared way belongs to more than two districts')
    require(any(count == 2 for count in counts.values()), 'union has no shared district edges')
    synthetic = dict(type='relation', id=-1, version=1,
                     tags=dict(type='boundary', wikidata='union', name=expected['label']),
                     members=[member for ref, member in members.items() if counts[ref] == 1])
    rings = join_rings([index['way', member['ref']]['nodes'] for member in synthetic['members']])
    coordinates = [[[index['node', node]['lon'], index['node', node]['lat']] for node in ring]
                   for ring in rings]
    require(not any(inside(ring[0], other) for i, ring in enumerate(coordinates)
                    for j, other in enumerate(coordinates) if i != j),
            'union encloses a hole or nested component; explicit topology is required')
    combined = json.dumps(dict(elements=list(index.values()) + [synthetic])).encode()
    spec = dict(expected, relation_id=-1, relation_version=1, wikidata='union')
    feature = json.loads(convert(combined, spec))
    feature['properties'] = dict(
        name=expected['label'],
        source_relations=[dict(relation_id=s['relation_id'], relation_version=s['relation_version'],
                               source_snapshot_sha256=digest(raw))
                          for s, raw in zip(expected['sources'], raws)],
        license='ODbL-1.0; © OpenStreetMap contributors', scope=expected['scope'],
        conversion='Adjacent district union by exact shared OSM way cancellation; exterior ways joined by node identity. No resampling, simplification or coordinate snapping.')
    return (json.dumps(feature, ensure_ascii=False, indent=2) + '\n').encode()


def run(check=False, manifest_path=MANIFEST):
    manifest_path = Path(manifest_path)
    manifest = json.loads(manifest_path.read_text())
    require(manifest['schema_version'] == 1, 'unknown boundary manifest schema')
    for boundary in manifest['boundaries']:
        sources = boundary.get('sources', [boundary])
        raws = []
        for source in sources:
            raw = pinned_file(source['source_snapshot'], manifest_path.parent).read_bytes()
            require(digest(raw) == source['source_sha256'], 'source snapshot checksum mismatch')
            raws.append(raw)
        output = convert_union(raws, boundary) if 'sources' in boundary else convert(raws[0], boundary)
        require(digest(output) == boundary['sha256'], 'derived boundary checksum mismatch')
        path = pinned_file(boundary['file'], manifest_path.parent)
        if check:
            require(path.read_bytes() == output, 'stale derived boundary: ' + str(path))
        else:
            path.write_bytes(output)
        print(f"{boundary['label']}: {'verified' if check else 'rebuilt'} "
              f"{boundary['components']} components, {boundary['rings']} rings, "
              f"{boundary['coordinate_pairs']} coordinates")


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', help='reproduce in memory and verify exact pinned bytes')
    parser.add_argument('--manifest', type=Path, default=MANIFEST, help='pinned boundary manifest')
    args = parser.parse_args()
    run(args.check, args.manifest)
