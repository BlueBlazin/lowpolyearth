#!/usr/bin/env python3
"""Pack full-precision timezone polygons for bounded-memory offline lookup.

The runtime keeps only the small polygon index resident. Each polygon is delta
encoded and compressed independently, so a lookup inflates only the handful of
candidate polygons whose bounding boxes contain the requested coordinate.
Coordinates are quantized to 1e-6 degrees.
"""

from __future__ import annotations

import argparse
import io
import json
import struct
import zipfile
import zlib
from pathlib import Path


MAGIC = b"VTZ1"
VERSION = 1
SCALE = 1_000_000


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


def read_document(source: Path) -> dict:
    if source.suffix.lower() != ".zip":
        return json.loads(source.read_text())
    with zipfile.ZipFile(source) as archive:
        members = [name for name in archive.namelist() if name.endswith((".json", ".geojson"))]
        if len(members) != 1:
            raise ValueError(f"expected one GeoJSON member, found {members}")
        with archive.open(members[0]) as stream:
            return json.load(io.TextIOWrapper(stream, encoding="utf-8"))


def encode_polygon(rings: list[list[tuple[int, int]]]) -> tuple[bytes, int]:
    raw = bytearray(struct.pack("<H", len(rings)))
    for ring in rings:
        raw.extend(struct.pack("<I", len(ring)))
        previous_x = previous_y = 0
        for x, y in ring:
            append_signed_varint(raw, x - previous_x)
            append_signed_varint(raw, y - previous_y)
            previous_x, previous_y = x, y
    return zlib.compress(bytes(raw), level=9), len(raw)


def append_signed_varint(output: bytearray, value: int) -> None:
    unsigned = (value << 1) ^ (value >> 31)
    while unsigned >= 0x80:
        output.append((unsigned & 0x7F) | 0x80)
        unsigned >>= 7
    output.append(unsigned)


def build(source: Path) -> bytes:
    document = read_document(source)
    names = set()
    pending_records = []
    for feature in document["features"]:
        timezone = str((feature.get("properties") or {}).get("tzid") or "").strip()
        if not timezone:
            continue
        names.add(timezone)
        for rings in feature_polygons(feature):
            points = [point for ring in rings for point in ring]
            bbox = (
                min(point[0] for point in points),
                min(point[1] for point in points),
                max(point[0] for point in points),
                max(point[1] for point in points),
            )
            compressed, raw_length = encode_polygon(rings)
            width = bbox[2] - bbox[0]
            if width > 180 * SCALE:
                width = 360 * SCALE - width
            area = max(width, 0) * max(bbox[3] - bbox[1], 0)
            pending_records.append((area, timezone, bbox, compressed, raw_length))
        # The parsed GeoJSON is already large. Drop processed coordinates so
        # packing never retains a second full-world representation.
        feature["geometry"] = None

    names = sorted(names)
    name_indexes = {name: index for index, name in enumerate(names)}
    if len(names) > 0xFFFF:
        raise ValueError("too many timezone names")

    records = []
    for area, name, bbox, compressed, raw_length in pending_records:
        records.append((area, name, name_indexes[name], bbox, compressed, raw_length))

    # When source polygons touch or overlap, prefer the geographically tighter
    # polygon. Stable name ordering makes an exact boundary deterministic.
    records.sort(key=lambda record: (record[0], record[1], record[3]))

    names_blob = bytearray()
    for name in names:
        encoded = name.encode("utf-8")
        if len(encoded) > 0xFFFF:
            raise ValueError(f"timezone name is too long: {name}")
        names_blob.extend(struct.pack("<H", len(encoded)))
        names_blob.extend(encoded)

    header_length = 4 + 4 * 4 + len(names_blob) + len(records) * 34
    offset = header_length
    index = bytearray()
    blocks = bytearray()
    for _, _, zone_index, bbox, compressed, raw_length in records:
        index.extend(
            struct.pack(
                "<HiiiiIIII",
                zone_index,
                *bbox,
                offset,
                len(compressed),
                raw_length,
                0,
            )
        )
        blocks.extend(compressed)
        offset += len(compressed)

    output = bytearray(MAGIC)
    output.extend(struct.pack("<IIII", VERSION, SCALE, len(names), len(records)))
    output.extend(names_blob)
    output.extend(index)
    output.extend(blocks)
    return bytes(output)


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
