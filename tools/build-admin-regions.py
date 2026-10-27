#!/usr/bin/env python3
"""Pack Admin-1 GeoJSON polygons for offline location labels.

The runtime needs an English-preferred display name, the containing country
name, a stable identifier, a bounding box and polygon rings. Coordinates are
quantized to 1e-5 degrees, delta encoded and zlib compressed.
"""

from __future__ import annotations

import argparse
import json
import struct
import zlib
from pathlib import Path


MAGIC = b"VAR1"
VERSION = 1
SCALE = 100_000


def quantize_ring(coordinates: list[list[float]]) -> list[tuple[int, int]]:
    points: list[tuple[int, int]] = []
    for longitude, latitude, *_ in coordinates:
        point = (round(longitude * SCALE), round(latitude * SCALE))
        if not points or point != points[-1]:
            points.append(point)
    if len(points) > 1 and points[0] == points[-1]:
        points.pop()
    return points if len(points) >= 3 else []


def feature_polygons(feature: dict) -> list[list[list[tuple[int, int]]]]:
    geometry = feature.get("geometry") or {}
    coordinates = geometry.get("coordinates") or []
    if geometry.get("type") == "Polygon":
        coordinates = [coordinates]
    elif geometry.get("type") != "MultiPolygon":
        return []

    polygons = []
    for polygon in coordinates:
        rings = [quantize_ring(ring) for ring in polygon]
        rings = [ring for ring in rings if ring]
        if rings:
            polygons.append(rings)
    return polygons


def stable_fallback_id(text: str) -> int:
    value = 0xCBF29CE484222325
    for byte in text.encode("utf-8"):
        value = ((value ^ byte) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return value


def build(source: Path) -> bytes:
    document = json.loads(source.read_text())
    regions = []
    for feature in document["features"]:
        properties = feature.get("properties") or {}
        name = (properties.get("name_en") or properties.get("name") or "").strip()
        if not name:
            continue
        country_name = (properties.get("admin") or "").strip()
        polygons = feature_polygons(feature)
        if not polygons:
            continue
        points = [point for polygon in polygons for ring in polygon for point in ring]
        bbox = (
            min(point[0] for point in points),
            min(point[1] for point in points),
            max(point[0] for point in points),
            max(point[1] for point in points),
        )
        source_key = str(
            properties.get("ne_id")
            or properties.get("adm1_code")
            or properties.get("iso_3166_2")
            or name
        )
        identifier = (
            int(properties["ne_id"])
            if properties.get("ne_id") is not None
            else stable_fallback_id(source_key)
        )
        regions.append((identifier, name, country_name, bbox, polygons))

    regions.sort(key=lambda region: (region[0], region[1]))
    output = bytearray(MAGIC)
    output.extend(struct.pack("<III", VERSION, SCALE, len(regions)))
    for identifier, name, country_name, bbox, polygons in regions:
        encoded_name = name.encode("utf-8")
        encoded_country = country_name.encode("utf-8")
        if (
            len(encoded_name) > 0xFFFF
            or len(encoded_country) > 0xFFFF
            or len(polygons) > 0xFFFF
        ):
            raise ValueError(f"region is too large to encode: {name}")
        output.extend(struct.pack("<QiiiiH", identifier, *bbox, len(encoded_name)))
        output.extend(encoded_name)
        output.extend(struct.pack("<H", len(encoded_country)))
        output.extend(encoded_country)
        output.extend(struct.pack("<H", len(polygons)))
        for polygon in polygons:
            if len(polygon) > 0xFFFF:
                raise ValueError(f"too many rings in {name}")
            output.extend(struct.pack("<H", len(polygon)))
            for ring in polygon:
                output.extend(struct.pack("<I", len(ring)))
                previous_x = previous_y = 0
                for x, y in ring:
                    output.extend(struct.pack("<ii", x - previous_x, y - previous_y))
                    previous_x, previous_y = x, y
    return zlib.compress(bytes(output), level=9)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    packed = build(args.source)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(packed)
    print(f"wrote {args.output} ({len(packed):,} bytes)")


if __name__ == "__main__":
    main()
