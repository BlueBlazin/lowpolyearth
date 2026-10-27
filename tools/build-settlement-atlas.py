#!/usr/bin/env python3
"""Losslessly pack the native GHS-SMOD 2020 1 km Mollweide category raster.

Requires numpy and rasterio. Input is the official R2023A V2-0 GeoTIFF;
output preserves every pixel, including water and no-data. Each row stores
exclusive run ends and signed source categories, preceded by a row index.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
from pathlib import Path

import numpy as np
import rasterio
from rasterio.windows import Window


MAGIC = b"VSM1"
VERSION = 1
EPOCH = 2020
RADIUS = 6_378_137.0
HEADER = struct.Struct("<4sIII5dII")
RUN = np.dtype([("end", "<u2"), ("category", "<i2")])
CATEGORIES = {-200, 10, 11, 12, 13, 21, 22, 23, 30}
SOURCE_URL = (
    "https://jeodpp.jrc.ec.europa.eu/ftp/jrc-opendata/GHSL/"
    "GHS_SMOD_GLOBE_R2023A/GHS_SMOD_E2020_GLOBE_R2023A_54009_1000/V2-0/"
    "GHS_SMOD_E2020_GLOBE_R2023A_54009_1000_V2_0.zip"
)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def encode_row(row: np.ndarray) -> np.ndarray:
    row = np.asarray(row)
    if row.ndim != 1 or not 0 < row.size <= 0xFFFF:
        raise ValueError("a row must contain 1..65535 pixels")
    values = set(map(int, np.unique(row)))
    if not values <= CATEGORIES:
        raise ValueError(f"unexpected source categories: {sorted(values - CATEGORIES)}")
    ends = np.append(np.flatnonzero(row[1:] != row[:-1]) + 1, row.size)
    runs = np.empty(ends.size, dtype=RUN)
    runs["end"] = ends
    runs["category"] = row[ends - 1]
    decoded = np.repeat(runs["category"], np.diff(np.append(0, ends)))
    if not np.array_equal(row, decoded):
        raise ValueError("row run encoding failed lossless verification")
    return runs


def build(source: Path, output: Path) -> dict:
    with rasterio.open(source) as raster:
        expected_transform = rasterio.Affine(1000, 0, -18_041_000, 0, -1000, 9_000_000)
        if (
            raster.crs != rasterio.CRS.from_string("ESRI:54009")
            or raster.transform != expected_transform
            or (raster.width, raster.height) != (36_082, 18_000)
            or raster.count != 1
            or raster.dtypes != ("int16",)
            or raster.nodata != -200
        ):
            raise ValueError("source is not the native global GHS-SMOD 1 km Mollweide grid")
        width, height = raster.width, raster.height
        header = HEADER.pack(
            MAGIC, VERSION, width, height,
            raster.transform.c, raster.transform.f,
            raster.transform.a, raster.transform.e,
            RADIUS, EPOCH, 0,
        )
        offsets = [0]
        categories = {category: 0 for category in sorted(CATEGORIES)}
        output.parent.mkdir(parents=True, exist_ok=True)
        with output.open("wb") as packed:
            packed.write(header)
            packed.write(bytes((height + 1) * 4))
            for block_start in range(0, height, 256):
                block = raster.read(
                    1, window=Window(0, block_start, width, min(256, height - block_start))
                )
                for row in block:
                    runs = encode_row(row)
                    packed.write(runs.tobytes())
                    offsets.append(offsets[-1] + len(runs))
                    lengths = np.diff(np.append(0, runs["end"].astype(np.int64)))
                    for category in categories:
                        categories[category] += int(lengths[runs["category"] == category].sum())
            packed.seek(HEADER.size)
            packed.write(np.asarray(offsets, dtype="<u4").tobytes())
    return {
        "format": "VSM1",
        "version": VERSION,
        "dataset": "GHS-SMOD R2023A V2-0, epoch 2020",
        "source_url": SOURCE_URL,
        "source_filename": source.name,
        "source_sha256": sha256(source),
        "source_crs": "ESRI:54009",
        "width": width,
        "height": height,
        "pixel_size_m": 1000,
        "epoch": EPOCH,
        "category_pixel_counts": categories,
        "run_count": offsets[-1],
        "output_bytes": output.stat().st_size,
        "output_sha256": sha256(output),
        "adaptation": "Lossless row-run encoding; no resampling or category changes.",
        "license": "CC BY 4.0",
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--manifest", type=Path)
    args = parser.parse_args()
    result = build(args.source, args.output)
    serialized = json.dumps(result, indent=2) + "\n"
    if args.manifest:
        args.manifest.write_text(serialized)
    print(serialized, end="")


if __name__ == "__main__":
    main()
