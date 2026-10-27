#!/usr/bin/env python3
"""Verify/rebuild Helsinki architecture masks from selected municipal features.

Requires Shapely 2 only for this offline authoring operation; normal atlas builds
read the checked GeoJSON directly. Source data: City of Helsinki, CC BY 4.0.
"""

import argparse
import hashlib
import json
from pathlib import Path

from shapely import remove_repeated_points
from shapely.geometry import mapping, shape
from shapely.ops import unary_union

ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / "crates/vearth-gen/assets/architecture/helsinki"


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def rebuild(root, check=False):
    manifest = json.loads((root / "sources.json").read_text())
    require(manifest["license"] == "CC BY 4.0", "Unexpected source license")
    for name, identities in manifest["selected_origin_ids"].items():
        require(Path(name).name == name, "Boundary name must be a local basename")
        source = name + "-source.geojson"
        raw = (root / source).read_bytes()
        require(digest(raw) == manifest["sha256"][source], f"Changed source: {source}")
        data = json.loads(raw)
        require(
            data.get("type") == "FeatureCollection" and "crs" not in data,
            "Expected WGS84 source features",
        )
        features = data["features"]
        require(
            [f["properties"]["origin_id"] for f in features] == identities,
            "Source identity mismatch",
        )
        geometries = [shape(f["geometry"]) for f in features]
        require(
            all(g.is_valid and g.geom_type in ("Polygon", "MultiPolygon") for g in geometries),
            "Invalid source municipal polygon",
        )
        geometry = unary_union(geometries)
        require(geometry.is_valid, "Invalid municipal union")
        result = {
            "type": "Feature",
            "properties": {
                "name": name,
                "selected_neighborhoods": [f["properties"]["name"]["fi"] for f in features],
                "attribution": manifest["attribution"],
                "license": manifest["license"],
            },
            "geometry": mapping(remove_repeated_points(geometry, tolerance=0)),
        }
        encoded = (json.dumps(result, ensure_ascii=False, separators=(",", ":")) + "\n").encode()
        output = name + ".geojson"
        require(digest(encoded) == manifest["sha256"][output], f"Conversion drift: {output}")
        if check:
            require((root / output).read_bytes() == encoded, f"Stale output: {output}")
        else:
            (root / output).write_bytes(encoded)
    print("Verified selected Helsinki source snapshots and derived envelopes.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-dir", type=Path, default=ASSETS)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    rebuild(args.source_dir, check=args.check)


if __name__ == "__main__":
    main()
