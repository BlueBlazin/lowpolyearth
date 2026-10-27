#!/usr/bin/env python3

"""Build the bundled runtime geometry from country and lake GeoJSON.

Usage:
  python3 tools/build_mercator_map.py countries.geojson lakes.geojson out.bin
"""

import json
import math
import struct
import sys
from pathlib import Path


SIZE = 2048
LAT_LIMIT = 85.05112878
def project(point):
    longitude, latitude = point[:2]
    lon = max(-180.0, min(180.0, longitude))
    lat = max(-LAT_LIMIT, min(LAT_LIMIT, latitude))
    radians = math.radians(lat)
    x = ((lon + 180.0) / 360.0) * SIZE
    y = (
        0.5
        - math.log(math.tan(math.pi / 4.0 + radians / 2.0)) / (2.0 * math.pi)
    ) * SIZE
    return (x, y)


def normalized(point):
    x, y = project(point)
    return (x / SIZE, y / SIZE)


def polygons(geometry):
    if not geometry:
        return []
    if geometry["type"] == "Polygon":
        return [geometry["coordinates"]]
    if geometry["type"] == "MultiPolygon":
        return geometry["coordinates"]
    return []


def encode_collection(output, collection):
    features = [
        polygons(feature.get("geometry")) for feature in collection["features"]
    ]
    features = [feature for feature in features if feature]
    output.write(struct.pack("<I", len(features)))
    for feature in features:
        output.write(struct.pack("<I", len(feature)))
        for polygon in feature:
            output.write(struct.pack("<I", len(polygon)))
            for ring in polygon:
                output.write(struct.pack("<I", len(ring)))
                for point in ring:
                    output.write(struct.pack("<ff", *normalized(point)))


def build_geometry(countries, lakes, output):
    with output.open("wb") as encoded:
        encoded.write(b"VEARTHM1")
        encode_collection(encoded, countries)
        encode_collection(encoded, lakes)


def main():
    if len(sys.argv) != 4:
        raise SystemExit("expected countries.geojson lakes.geojson output.bin")
    countries = json.loads(Path(sys.argv[1]).read_text())
    lakes = json.loads(Path(sys.argv[2]).read_text())
    build_geometry(countries, lakes, Path(sys.argv[3]))


if __name__ == "__main__":
    main()
