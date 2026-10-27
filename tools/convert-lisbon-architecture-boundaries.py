#!/usr/bin/env python3
"""Rebuild or verify selected Lisbon municipal planning boundaries.

The pinned ArcGIS GeoJSON response supplies WGS84 longitude/latitude pairs.
Selection preserves the complete feature; it does not reproject or simplify it.
"""

import argparse
import hashlib
import json
import math
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / "crates/vearth-gen/assets/architecture/lisbon"
MANIFEST = ASSETS / "boundaries.source.json"


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def pinned_file(name):
    require(isinstance(name, str) and Path(name).name == name,
            "source and output filenames must be local basenames")
    path = ASSETS / name
    require(path.resolve().is_relative_to(ASSETS.resolve()), "asset path escapes directory")
    return path


def convert(raw, expected):
    require(digest(raw) == expected["source_sha256"], "source snapshot checksum mismatch")
    data = json.loads(raw)
    require(data.get("type") == "FeatureCollection", "expected municipal FeatureCollection")
    require(data.get("crs") == {"type": "name", "properties": {"name": "EPSG:4326"}},
            "source must declare EPSG:4326")
    require(not data.get("exceededTransferLimit"), "truncated municipal response")
    selector = expected["selector"]
    require(bool(selector), "selected feature needs an explicit identity")
    matches = [feature for feature in data["features"]
               if all(feature.get("properties", {}).get(key) == value
                      for key, value in selector.items())]
    require(len(matches) == 1, "selected feature identity must occur exactly once")
    feature = matches[0]
    require(feature.get("type") == "Feature" and "crs" not in feature,
            "expected one unambiguous Feature")
    properties = feature["properties"]
    require(properties.get("NOME") == expected["source_name"], "selected plan name changed")
    require(properties.get("ESTADO") == "Eficaz" and properties.get("ATIVO") == "S",
            "selected plan is no longer active and effective")
    require(properties.get("DATAPUBLICACAO") == expected["publication_date"],
            "selected plan publication changed")
    geometry = feature["geometry"]
    require("crs" not in geometry and geometry.get("type") in ("Polygon", "MultiPolygon"),
            "selected feature must contain Polygon or MultiPolygon")
    components = ([geometry["coordinates"]] if geometry["type"] == "Polygon"
                  else geometry["coordinates"])
    require(len(components) == expected["components"], "component count changed")
    require(sum(map(len, components)) == expected["rings"], "ring count changed")
    require(sum(len(ring) for component in components for ring in component)
            == expected["coordinate_pairs"], "coordinate count changed")
    for component in components:
        require(bool(component), "component must contain an exterior")
        for ring in component:
            require(len(ring) >= 4 and ring[0] == ring[-1], "source rings must be closed")
            require(all(isinstance(point, list) and len(point) == 2
                        and all(type(v) in (int, float) and math.isfinite(v) for v in point)
                        and abs(point[0]) <= 180 and abs(point[1]) <= 90 for point in ring),
                    "invalid WGS84 coordinate")
            require(all(a != b for a, b in zip(ring, ring[1:])), "duplicate consecutive points")
    return (json.dumps(feature, ensure_ascii=False, indent=2) + "\n").encode()


def run(check=False):
    manifest = json.loads(MANIFEST.read_text())
    require(manifest["schema_version"] == 1, "unknown boundary manifest schema")
    for boundary in manifest["boundaries"]:
        output = convert(pinned_file(boundary["source_snapshot"]).read_bytes(), boundary)
        require(digest(output) == boundary["sha256"], "derived boundary checksum mismatch")
        path = pinned_file(boundary["file"])
        if check:
            require(path.read_bytes() == output, "stale derived boundary: " + str(path))
        else:
            path.write_bytes(output)
        print(f"{boundary['label']}: {'verified' if check else 'rebuilt'} "
              f"{boundary['components']} components, {boundary['rings']} rings, "
              f"{boundary['coordinate_pairs']} coordinates")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="reproduce in memory and verify exact pinned bytes")
    run(parser.parse_args().check)
