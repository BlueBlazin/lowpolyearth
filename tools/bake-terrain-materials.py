#!/usr/bin/env python3
"""Bake four linear data tiles; requires NumPy and Pillow, never runs at startup."""

import argparse
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / "crates/vearth-app/assets/materials"
SIDE = 128
NAMES = ("sand", "grass", "rock", "snow")


def blur(field, radius):
    # A periodic Gaussian has no special edge pixels or derivative join.
    f = np.fft.fftfreq(SIDE)
    kernel = np.exp(-2 * np.pi**2 * radius**2 * (f[:, None] ** 2 + f[None, :] ** 2))
    return np.fft.ifft2(np.fft.fft2(field) * kernel).real


def noise(rng, radius):
    field = blur(rng.uniform(-1, 1, (SIDE, SIDE)), radius)
    return (field - field.mean()) / field.std()


def periodic_source(source):
    # Separate the boundary discontinuity from the painted image by solving
    # the periodic discrete Poisson equation. Unlike copying opposing pixels,
    # this does not leave a one-texel plateau in the height derivative.
    boundary = np.zeros_like(source)
    boundary[0, :] = source[-1, :] - source[0, :]
    boundary[-1, :] = -boundary[0, :]
    boundary[:, 0] += source[:, -1] - source[:, 0]
    boundary[:, -1] -= source[:, -1] - source[:, 0]
    c = np.cos(2 * np.pi * np.fft.fftfreq(SIDE))
    eigenvalue = 2 * c[:, None] + 2 * c[None, :] - 4
    eigenvalue[0, 0] = 1
    correction = np.fft.fft2(boundary) / eigenvalue
    correction[0, 0] = 0
    return blur(source - np.fft.ifft2(correction).real, 0.65)


def author():
    settings = json.loads((ASSETS / "terrain-art.json").read_text())
    provenance = json.loads((ASSETS / "stone-tone.source.json").read_text())
    original = (ASSETS / "stone-tone.png").read_bytes()
    assert hashlib.sha256(original).hexdigest() == settings["stone_source_sha256"]
    assert settings["stone_source_sha256"] == provenance["asset_sha256"]
    rng = np.random.Generator(np.random.PCG64(settings["seed"]))
    y, x = np.mgrid[:SIDE, :SIDE] / SIDE + 0.5 / SIDE
    broad, medium, fine = (noise(rng, r) for r in (10, 3, 0.8))

    # Sand aggregates have several sizes, without a ripple axis. Grain is
    # chiefly tonal/roughness structure; its height is only millimetres.
    sand = (0.025 * medium + 0.010 * fine + 0.009 * broad,
            0.014 * broad + 0.004 * medium + 0.0007 * blur(fine, 0.6),
            0.956 + 0.008 * medium + 0.005 * fine)

    # Overlapping, individually oriented tufts with lobed outlines. Colour
    # varies between tufts independently of their elevation.
    warp_x, warp_y = (noise(rng, 7) * 0.012 for _ in range(2))
    cover, pigment = np.zeros_like(x), np.zeros_like(x)
    for _ in range(68):
        cx, cy = rng.random(2)
        rx, ry = rng.uniform(0.017, 0.047, 2)
        angle = rng.uniform(0, 2 * np.pi)
        dx = (x + warp_x - cx + 0.5) % 1 - 0.5
        dy = (y + warp_y - cy + 0.5) % 1 - 0.5
        u = (dx * np.cos(angle) + dy * np.sin(angle)) / rx
        v = (-dx * np.sin(angle) + dy * np.cos(angle)) / ry
        tuft = np.exp(-1.6 * (u * u + v * v))
        # Unequal side lobes break the oval into a small, fanned clump.
        tuft += 0.55 * np.exp(-3.5 * ((u - 0.55)**2 + (v + 0.30)**2))
        tuft += 0.35 * np.exp(-4.0 * ((u + 0.40)**2 + (v - 0.45)**2))
        cover += tuft
        pigment += tuft * rng.uniform(-0.5, 0.5)
    cover = 1 - np.exp(-cover * 1.5)
    grass = (0.27 * cover + 0.040 * pigment + 0.009 * medium,
             0.049 * blur(cover, 0.8) + 0.005 * broad + 0.001 * medium,
             0.961 - 0.045 * cover + 0.008 * pigment)

    with Image.open(ASSETS / "stone-tone.png") as image:
        assert list(image.size) == provenance["actual_dimensions"]
        assert image.width == image.height
        rgb = np.asarray(image.convert("RGB"), dtype=np.float64) / 255
    luminance = rgb @ np.array([0.2126, 0.7152, 0.0722])
    # Exact area integration, including partial source pixels at 1254/128.
    edges = np.arange(SIDE + 1) * luminance.shape[0] / SIDE
    pixels = np.arange(luminance.shape[0])
    weights = np.maximum(0, np.minimum(edges[1:, None], pixels + 1)
                         - np.maximum(edges[:-1, None], pixels))
    weights /= weights.sum(axis=1, keepdims=True)
    stone = periodic_source(weights @ luminance @ weights.T)
    distance = np.full_like(x, 1.0)
    # Explicitly traced major joints in the retained source art. Brightness
    # remains pigment, never a height measurement. Wrapped copies join the
    # fracture network through the tile boundaries.
    for line in settings["fractures"]:
        for a, b in zip(line, line[1:]):
            a, b = np.array(a), np.array(b)
            delta = b - a
            for oy in (-1, 0, 1):
                for ox in (-1, 0, 1):
                    dx, dy = x - a[0] - ox, y - a[1] - oy
                    t = np.clip((dx * delta[0] + dy * delta[1]) / (delta @ delta), 0, 1)
                    distance = np.minimum(distance, np.hypot(dx - t * delta[0], dy - t * delta[1]))
    joint = blur(np.exp(-(distance / 0.0065)**2), 0.75)
    shoulder = blur(1 - np.exp(-distance / 0.027), 1.2)
    rock = ((stone - stone.mean()) * 1.90 - 0.026 * joint + 0.004 * medium,
            0.047 * shoulder - 0.020 * joint + 0.009 * noise(rng, 6),
            0.888 + 0.045 * joint + 0.008 * medium)

    # Soft, shallow settled snow; no crystalline points or rock-like joints.
    soft, powder = noise(rng, 5), noise(rng, 1.5)
    snow = (0.008 * soft + 0.003 * broad + 0.0015 * powder,
            0.009 * soft + 0.005 * broad + 0.0005 * powder,
            0.715 + 0.013 * soft + 0.004 * powder)
    return (sand, grass, rock, snow)


def encode(fields):
    tone, height, roughness = fields
    # H is an artistic relief in metres. Central dH/dUV divided by four:
    # (H[x+1]-H[x-1]) * 128/8. shader.wgsl applies 4/period_m,
    # followed by its existing 1.5 artistic strength.
    gradients = [(np.roll(height, -1, axis) - np.roll(height, 1, axis)) * SIDE / 8
                 for axis in (1, 0)]
    assert max(abs(g).max() for g in gradients) < 0.49, "height clips the gradient encoding"
    linear = np.stack((0.5 + tone - tone.mean(), 0.5 + gradients[0],
                       0.5 + gradients[1], roughness), axis=-1)
    assert np.isfinite(linear).all() and linear.min() > 0 and linear.max() < 1
    return np.floor(linear * 255 + 0.5).astype(np.uint8)


def validate(fields, tile):
    assert abs(tile[:, :, 0].mean() - 127.5) < 0.02
    for axis in (0, 1):
        # A wrap boundary must be an ordinary neighboring sample, including
        # the encoded derivative. Identical edge texels are not required.
        delta = abs(tile.astype(float) - np.roll(tile.astype(float), 1, axis))
        seam = np.take(delta, 0, axis=axis).mean(axis=0)
        assert (seam < delta.mean(axis=(0, 1)) * 3 + 1).all(), (seam, delta.mean(axis=(0, 1)))
    # Quantized derivatives must still be integrable (mixed partials commute).
    gx, gy = tile[:, :, 1:3].astype(float).transpose(2, 0, 1)
    curl = np.roll(gx, -1, 0) - np.roll(gx, 1, 0) - np.roll(gy, -1, 1) + np.roll(gy, 1, 1)
    assert abs(curl).max() <= 2
    assert abs(np.corrcoef(fields[0].flat, fields[1].flat)[0, 1]) < 0.995


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="verify deterministic bytes and provenance without writing")
    parser.add_argument("--preview", type=Path, help="write inspectable tone/height/roughness images outside runtime assets")
    args = parser.parse_args()
    authored = author()
    tiles = [encode(fields) for fields in authored]
    for fields, tile in zip(authored, tiles):
        validate(fields, tile)
    data = b"".join(tile.tobytes() for tile in tiles)
    manifest = {
        "schema": 1, "asset": "terrain-tiles.rgba8", "layers": dict(zip(NAMES, range(2, 6))),
        "dimensions": [SIDE, SIDE], "bytes": len(data),
        "encoding": "linear R=0.5+zero-mean tone; GB=0.5+central dH/dUV/4; A=perceptual roughness",
        "mips": "base only; atlas::build retains production box filtering for all eight levels",
        "bake": "python3 tools/bake-terrain-materials.py",
        "sources": {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest()
                    for path in (Path(__file__), ASSETS / "terrain-art.json", ASSETS / "stone-tone.png",
                                 ASSETS / "stone-tone.source.json")},
        "sha256": hashlib.sha256(data).hexdigest(),
        "provenance": "Deterministic illustrative art, not measured material data. Rock tone retains stone-tone.png; hand-traced joints guide independent metric relief. Original PNG and attribution retained."
    }
    outputs = {ASSETS / "terrain-tiles.rgba8": data,
               ASSETS / "terrain-tiles.source.json": (json.dumps(manifest, indent=2) + "\n").encode()}
    for path, content in outputs.items():
        if args.check:
            assert path.read_bytes() == content, f"stale bake: {path}"
        else:
            path.write_bytes(content)
    if args.preview:
        args.preview.mkdir(parents=True, exist_ok=True)
        for name, fields, tile in zip(NAMES, authored, tiles):
            for signal, values in zip(("tone", "height", "roughness"), fields):
                if signal == "tone":
                    pixels = tile[:, :, 0]
                elif signal == "height":
                    pixels = np.floor((values - values.min()) / np.ptp(values) * 255 + 0.5).astype(np.uint8)
                else:
                    pixels = tile[:, :, 3]
                Image.fromarray(pixels).save(args.preview / f"{name}-{signal}.png")
    print(f"{'Verified' if args.check else 'Baked'} {len(data):,} bytes; layers 2–5; all authoring invariants passed")


if __name__ == "__main__":
    main()
