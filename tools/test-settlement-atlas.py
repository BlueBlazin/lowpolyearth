#!/usr/bin/env python3
"""Check the settlement encoder and optionally verify the native source raster.

Run with --source TIFF for independent GDAL/PROJ checks of the bundled atlas.
Requires numpy, rasterio and (for source verification) pyproj.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import math
import mmap
import random
import sys
import unittest
from pathlib import Path

import numpy as np


ROOT = Path(__file__).resolve().parents[1]
sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("builder", Path(__file__).with_name("build-settlement-atlas.py"))
BUILDER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BUILDER)


class EncoderTests(unittest.TestCase):
    def test_all_categories_and_last_pixel_roundtrip(self):
        row = np.asarray([-200, -200, 10, 11, 11, 12, 13, 21, 22, 23, 30], dtype=np.int16)
        runs = BUILDER.encode_row(row)
        self.assertEqual(runs["end"].tolist(), [2, 3, 5, 6, 7, 8, 9, 10, 11])
        self.assertEqual(runs["category"].tolist(), [-200, 10, 11, 12, 13, 21, 22, 23, 30])
        self.assertEqual(runs.dtype.itemsize, 4)

    def test_uniform_maximum_width_and_single_pixel(self):
        for width in [1, 36_082, 65_535]:
            runs = BUILDER.encode_row(np.full(width, 30, dtype=np.int16))
            self.assertEqual(runs["end"].tolist(), [width])
            self.assertEqual(runs["category"].tolist(), [30])

    def test_unknown_codes_and_invalid_dimensions_are_rejected(self):
        for row in [np.array([], dtype=np.int16), np.array([99]), np.array([[30]]),
                    np.zeros(65_536, dtype=np.int16)]:
            with self.assertRaises(ValueError):
                BUILDER.encode_row(row)


def verify_source(source: Path, packed_path: Path) -> None:
    import pyproj
    import rasterio

    manifest = json.loads((ROOT / "crates/vearth-data/assets/settlement-2020-v1.source.json").read_text())
    reference = json.loads((ROOT / "crates/vearth-data/testdata/settlement-reference-samples.json").read_text())
    assert BUILDER.sha256(source) == manifest["source_sha256"] == reference["source_sha256"]
    assert BUILDER.sha256(packed_path) == manifest["output_sha256"]
    with packed_path.open("rb") as packed_file, mmap.mmap(packed_file.fileno(), 0, access=mmap.ACCESS_READ) as packed, rasterio.open(source) as raster:
        _, version, width, height, origin_x, origin_y, dx, dy, radius, epoch, _ = BUILDER.HEADER.unpack_from(packed)
        assert (version, width, height, dx, dy, radius, epoch) == (1, raster.width, raster.height, 1000, -1000, 6_378_137, 2020)
        row_offsets = np.ndarray((height + 1,), dtype="<u4", buffer=packed, offset=BUILDER.HEADER.size)
        runs_offset = BUILDER.HEADER.size + (height + 1) * 4

        def pixel(column, row):
            runs = np.ndarray((int(row_offsets[row + 1] - row_offsets[row]),), dtype=BUILDER.RUN,
                              buffer=packed, offset=runs_offset + int(row_offsets[row]) * 4)
            index = np.searchsorted(runs["end"], column, side="right")
            return int(runs["category"][index])

        transform = pyproj.Transformer.from_crs("EPSG:4326", raster.crs, always_xy=True)
        for sample in reference["samples"]:
            x, y = transform.transform(sample["lon_deg"], sample["lat_deg"])
            assert abs(x - sample["x_m"]) < 0.001 and abs(y - sample["y_m"]) < 0.001
            row, column = raster.index(x, y)
            assert (row, column) == (sample["row"], sample["column"])
            expected = int(next(raster.sample([(x, y)]))[0])
            assert expected == sample["raw_code"] == pixel(column, row)

        random_generator = random.Random(2020)
        positions = [(random_generator.uniform(-180, 180), random_generator.uniform(-85, 85)) for _ in range(10_000)]
        coordinates = [transform.transform(lon, lat) for lon, lat in positions]
        for (x, y), source_pixel in zip(coordinates, raster.sample(coordinates)):
            column, row = math.floor((x - origin_x) / dx), math.floor((y - origin_y) / dy)
            assert pixel(column, row) == int(source_pixel[0])
        del row_offsets
    print(f"Verified {len(reference['samples'])} reference locations and 10,000 global source samples")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path)
    parser.add_argument("--packed", type=Path, default=ROOT / "crates/vearth-data/assets/settlement-2020-v1.bin")
    args = parser.parse_args()
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(EncoderTests)
    result = unittest.TextTestRunner().run(suite)
    if not result.wasSuccessful():
        raise SystemExit(1)
    if args.source:
        verify_source(args.source, args.packed)


if __name__ == "__main__":
    main()
