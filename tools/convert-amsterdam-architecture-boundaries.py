#!/usr/bin/env python3
"""Rebuild or verify selected municipal Amsterdam boundary snapshots.

The pinned WFS response already contains CRS84 longitude/latitude coordinates.
Selection retains all feature properties and coordinates; it is not a reprojection.
"""

import argparse
import copy
import hashlib
import json
import math
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / "crates/vearth-gen/assets/architecture/amsterdam"
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
    require(data.get("crs") == {"type": "name", "properties": {
        "name": "urn:ogc:def:crs:OGC::CRS84"}}, "source must explicitly declare CRS84")
    require(data.get("timeStamp") == expected["source_timestamp"], "source timestamp changed")
    matches = [feature for feature in data["features"] if feature.get("id") == expected["feature_id"]]
    require(len(matches) == 1, "selected feature identity must occur exactly once")
    feature = copy.deepcopy(matches[0])
    require(feature.get("type") == "Feature" and "crs" not in feature,
            "expected one unambiguous Feature")
    properties = feature["properties"]
    require(properties.get("naam") == expected["label"], "selected boundary name changed")
    require(properties.get("aanwijzingsdatum") == expected["designation_date"],
            "selected designation date changed")
    require(properties.get("intrekkingsdatum") is None, "selected designation withdrawn")
    require("provenance" not in properties, "source already contains provenance field")
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
    properties["provenance"] = {
        "source_url": expected["source_url"],
        "source_snapshot_sha256": digest(raw),
        "source_timestamp": data["timeStamp"],
        "attribution": expected["attribution"],
        "license": expected["license"],
        "license_documentation": expected["license_documentation"],
        "conversion": f"Select feature {expected['feature_id']} from municipal WFS response requested in CRS84. Preserve all coordinates and source properties; remove collection wrapper only. No local reprojection, simplification or resampling.",
    }
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
