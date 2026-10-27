#!/usr/bin/env python3
"""Build the compact, offline Climate Atlas v2 used by Low Poly Earth.

Inputs are local files and are never fetched by the application:

* Twelve 2160 × 1080 monthly TIFFs for each of temperature, vapour pressure,
  and rainfall.
* Monthly wind and relative-humidity fields in HDF5 files.
* A VNLT v1 night-light proxy.

The output is a zlib-compressed, one-degree, twelve-month raster.  It is small
enough to include in native builds and contains only the
derived values needed by the runtime, not the source rasters.
"""

from __future__ import annotations

import argparse
import math
import struct
import zlib
from pathlib import Path

import h5py
import numpy as np
from PIL import Image


WIDTH = 360
HEIGHT = 180
MONTHS = 12
STRIDE = 19
MAGIC = b"VCL2"
VERSION = 2


def worldclim_month(root: Path, variable: str, month: int) -> np.ndarray:
    path = root / variable / f"wc2.1_10m_{variable}_{month:02d}.tif"
    with Image.open(path) as image:
        values = np.array(image, dtype=np.float32, copy=True)
    if values.shape != (1080, 2160):
        raise ValueError(f"unexpected WorldClim dimensions in {path}: {values.shape}")
    values[values < -1.0e20] = np.nan
    # Exactly six 10-minute cells form one degree.  Mean aggregation avoids
    # selecting a single coastal or mountain pixel at the runtime scale.
    return np.nanmean(values.reshape(HEIGHT, 6, WIDTH, 6), axis=(1, 3))


def bilinear_global(values: np.ndarray, source_lat: np.ndarray, source_lon: np.ndarray) -> np.ndarray:
    """Resample a descending-latitude, 0..360 periodic field to 1 degree."""
    target_lat = 89.5 - np.arange(HEIGHT, dtype=np.float32)
    target_lon = (np.arange(WIDTH, dtype=np.float32) + 180.5) % 360.0

    lat_ascending = source_lat[0] < source_lat[-1]
    if not lat_ascending:
        source_lat = source_lat[::-1]
        values = values[::-1, :]

    yi = np.searchsorted(source_lat, target_lat, side="right") - 1
    yi = np.clip(yi, 0, len(source_lat) - 2)
    y1 = yi + 1
    ty = (target_lat - source_lat[yi]) / np.maximum(source_lat[y1] - source_lat[yi], 1.0e-6)

    spacing = float(source_lon[1] - source_lon[0])
    x = (target_lon - float(source_lon[0])) / spacing
    x0 = np.floor(x).astype(np.int32) % len(source_lon)
    x1 = (x0 + 1) % len(source_lon)
    tx = x - np.floor(x)

    north = values[yi[:, None], x0[None, :]] * (1.0 - tx[None, :]) \
        + values[yi[:, None], x1[None, :]] * tx[None, :]
    south = values[y1[:, None], x0[None, :]] * (1.0 - tx[None, :]) \
        + values[y1[:, None], x1[None, :]] * tx[None, :]
    return north * (1.0 - ty[:, None]) + south * ty[:, None]


def read_noaa(path: Path, variable: str, level_hpa: float | None = None) -> list[np.ndarray]:
    with h5py.File(path, "r") as dataset:
        lat = np.asarray(dataset["lat"], dtype=np.float32)
        lon = np.asarray(dataset["lon"], dtype=np.float32)
        values = np.asarray(dataset[variable], dtype=np.float32)
        if level_hpa is not None:
            levels = np.asarray(dataset["level"], dtype=np.float32)
            level_index = int(np.argmin(np.abs(levels - level_hpa)))
            if abs(float(levels[level_index]) - level_hpa) > 1.0:
                raise ValueError(f"{level_hpa} hPa absent from {path}")
            values = values[:, level_index, :, :]
    return [bilinear_global(values[month], lat, lon) for month in range(MONTHS)]


def load_night_lights(path: Path) -> np.ndarray:
    data = path.read_bytes()
    if data[:4] != b"VNLT":
        raise ValueError(f"invalid Black Marble proxy: {path}")
    version, width, height = struct.unpack_from("<III", data, 4)
    if version != 1 or len(data) != 16 + width * height:
        raise ValueError(f"unsupported Black Marble proxy: {path}")
    source = np.frombuffer(data, dtype=np.uint8, offset=16).reshape(height, width).astype(np.float32) / 255.0
    target_y = (np.arange(HEIGHT, dtype=np.float32) + 0.5) * height / HEIGHT - 0.5
    target_x = (np.arange(WIDTH, dtype=np.float32) + 0.5) * width / WIDTH - 0.5
    y0 = np.clip(np.floor(target_y).astype(np.int32), 0, height - 1)
    y1 = np.clip(y0 + 1, 0, height - 1)
    x0 = np.floor(target_x).astype(np.int32) % width
    x1 = (x0 + 1) % width
    ty = target_y - np.floor(target_y)
    tx = target_x - np.floor(target_x)
    a = source[y0[:, None], x0[None, :]] * (1.0 - tx[None, :]) + source[y0[:, None], x1[None, :]] * tx[None, :]
    b = source[y1[:, None], x0[None, :]] * (1.0 - tx[None, :]) + source[y1[:, None], x1[None, :]] * tx[None, :]
    return a * (1.0 - ty[:, None]) + b * ty[:, None]


def gaussian_region(lat: np.ndarray, lon: np.ndarray, center_lat: float, center_lon: float,
                    lat_sigma: float, lon_sigma: float) -> np.ndarray:
    delta_lon = (lon - center_lon + 180.0) % 360.0 - 180.0
    return np.exp(-0.5 * (((lat - center_lat) / lat_sigma) ** 2 + (delta_lon / lon_sigma) ** 2))


def u8_unit(values: np.ndarray) -> np.ndarray:
    return np.rint(np.clip(values, 0.0, 1.0) * 255.0).astype(np.uint8)


def i8_half_mps(values: np.ndarray) -> np.ndarray:
    return np.rint(np.clip(values, -63.5, 63.5) * 2.0).astype(np.int8)


def build(args: argparse.Namespace) -> bytes:
    noaa_u_surface = read_noaa(args.uwnd_surface, "uwnd")
    noaa_v_surface = read_noaa(args.vwnd_surface, "vwnd")
    noaa_u_low = read_noaa(args.uwnd_pressure, "uwnd", 850.0)
    noaa_v_low = read_noaa(args.vwnd_pressure, "vwnd", 850.0)
    noaa_u_mid = read_noaa(args.uwnd_pressure, "uwnd", 700.0)
    noaa_v_mid = read_noaa(args.vwnd_pressure, "vwnd", 700.0)
    noaa_u_upper = read_noaa(args.uwnd_pressure, "uwnd", 300.0)
    noaa_v_upper = read_noaa(args.vwnd_pressure, "vwnd", 300.0)
    rh_low = read_noaa(args.rhum_pressure, "rhum", 850.0)
    rh_mid = read_noaa(args.rhum_pressure, "rhum", 700.0)
    rh_upper = read_noaa(args.rhum_pressure, "rhum", 300.0)
    lights = load_night_lights(args.night_lights)

    latitude = 89.5 - np.arange(HEIGHT, dtype=np.float32)
    longitude = -179.5 + np.arange(WIDTH, dtype=np.float32)
    lat2 = latitude[:, None]
    lon2 = longitude[None, :]

    output = bytearray()
    for month in range(MONTHS):
        temperature = worldclim_month(args.worldclim, "tavg", month + 1)
        vapour_kpa = worldclim_month(args.worldclim, "vapr", month + 1)
        precipitation_mm = worldclim_month(args.worldclim, "prec", month + 1)
        land = np.isfinite(temperature)

        # WorldClim is land-only. Ocean values retain physically smooth broad
        # normals; coastal interpolation in the runtime blends them naturally.
        seasonal = math.cos((month + 0.5) / 12.0 * math.tau)
        ocean_temperature = 27.5 - np.abs(lat2) * 0.43 + seasonal * np.sin(np.radians(lat2)) * -7.0
        temperature = np.where(land, temperature, ocean_temperature)
        saturation_kpa = 0.6108 * np.exp(17.27 * temperature / np.maximum(temperature + 237.3, 20.0))
        ocean_rh = np.clip(0.80 - np.abs(lat2) / 900.0, 0.66, 0.84)
        relative_humidity = np.where(land, vapour_kpa / np.maximum(saturation_kpa, 0.05), ocean_rh)
        relative_humidity = np.clip(relative_humidity, 0.08, 1.0)

        ocean_precip = 28.0 + 190.0 * np.exp(-((np.abs(lat2) / 17.0) ** 2)) \
            + 55.0 * np.exp(-(((np.abs(lat2) - 48.0) / 14.0) ** 2))
        precipitation_mm = np.where(land, precipitation_mm, ocean_precip)
        precipitation_mm = np.clip(precipitation_mm, 0.0, 4000.0)

        # Wet-hour occurrence is calibrated from the observed monthly total,
        # humidity and lower-tropospheric saturation. Conditional rain rate is
        # derived at runtime so monthly accumulation remains the authority.
        saturation_signal = np.clip((rh_low[month] - 58.0) / 38.0, 0.0, 1.0)
        wet_probability = precipitation_mm / (precipitation_mm + 620.0)
        wet_probability *= 0.62 + 0.38 * saturation_signal
        wet_probability = np.clip(wet_probability, 0.002, 0.38)

        low_cloud = np.clip((rh_low[month] - 48.0) / 47.0, 0.0, 1.0)
        mid_cloud = np.clip((rh_mid[month] - 42.0) / 50.0, 0.0, 1.0)
        high_cloud = np.clip((rh_upper[month] - 28.0) / 57.0, 0.0, 1.0)
        rain_cloud = np.clip(np.sqrt(precipitation_mm / 360.0), 0.0, 1.0)
        low_cloud = np.clip(low_cloud * 0.72 + rain_cloud * 0.28, 0.02, 0.98)
        mid_cloud = np.clip(mid_cloud * 0.82 + rain_cloud * 0.18, 0.01, 0.92)
        high_cloud = np.clip(high_cloud * 0.88 + rain_cloud * 0.12, 0.01, 0.88)

        dry = np.clip(1.0 - relative_humidity, 0.0, 1.0)
        dust_belt = np.exp(-0.5 * ((np.abs(lat2) - 24.0) / 10.0) ** 2) * dry * land
        urban = np.power(np.clip(lights, 0.0, 1.0), 0.34)
        # Regional aerosol priors supplement the night-light proxy.
        india = gaussian_region(lat2, lon2, 27.0, 79.0, 5.5, 10.0)
        china = gaussian_region(lat2, lon2, 34.0, 113.0, 7.0, 12.0)
        persian = gaussian_region(lat2, lon2, 27.0, 50.0, 7.0, 14.0)
        winter_north = 0.55 + 0.45 * max(math.cos((month + 0.5) / 12.0 * math.tau), 0.0)
        pollution = urban * 0.18 + india * 0.34 * winter_north + china * 0.23 * winter_north + persian * 0.12
        aerosol = np.clip(0.025 + dust_belt * 0.34 + pollution, 0.015, 1.05)
        dust_fraction = np.clip(dust_belt * 0.90 + persian * 0.42, 0.0, 1.0)

        fields = [
            np.rint(np.clip(temperature, -64.0, 63.5) * 2.0).astype(np.int8).view(np.uint8),
            u8_unit(relative_humidity),
            np.rint(precipitation_mm * 2.0).astype("<u2"),
            u8_unit(wet_probability),
            u8_unit(low_cloud),
            u8_unit(mid_cloud),
            u8_unit(high_cloud),
            i8_half_mps(noaa_u_surface[month]).view(np.uint8),
            i8_half_mps(noaa_v_surface[month]).view(np.uint8),
            i8_half_mps(noaa_u_low[month]).view(np.uint8),
            i8_half_mps(noaa_v_low[month]).view(np.uint8),
            i8_half_mps(noaa_u_mid[month]).view(np.uint8),
            i8_half_mps(noaa_v_mid[month]).view(np.uint8),
            i8_half_mps(noaa_u_upper[month]).view(np.uint8),
            i8_half_mps(noaa_v_upper[month]).view(np.uint8),
            u8_unit(aerosol / 1.2),
            u8_unit(dust_fraction),
            u8_unit(land.astype(np.float32)),
        ]

        # Interleave explicitly; precipitation occupies two bytes.
        cells = np.empty((HEIGHT, WIDTH, STRIDE), dtype=np.uint8)
        cells[:, :, 0] = fields[0]
        cells[:, :, 1] = fields[1]
        precip_bytes = fields[2].view(np.uint8).reshape(HEIGHT, WIDTH, 2)
        cells[:, :, 2:4] = precip_bytes
        for field_index, target in enumerate(range(4, STRIDE), start=3):
            cells[:, :, target] = fields[field_index]
        output.extend(cells.tobytes())

    header = struct.pack("<4sIHHBBH", MAGIC, VERSION, WIDTH, HEIGHT, MONTHS, STRIDE, 0)
    return header + zlib.compress(bytes(output), level=9)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--worldclim", type=Path, required=True)
    parser.add_argument("--uwnd-surface", type=Path, required=True)
    parser.add_argument("--vwnd-surface", type=Path, required=True)
    parser.add_argument("--uwnd-pressure", type=Path, required=True)
    parser.add_argument("--vwnd-pressure", type=Path, required=True)
    parser.add_argument("--rhum-pressure", type=Path, required=True)
    parser.add_argument("--night-lights", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    payload = build(args)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(payload)
    print(f"wrote {args.output} ({len(payload) / 1048576:.2f} MiB)")


if __name__ == "__main__":
    main()
