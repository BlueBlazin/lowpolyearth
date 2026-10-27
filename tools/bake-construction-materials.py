#!/usr/bin/env python3
"""Author construction tiles offline; NumPy and Pillow are bake-only dependencies."""

import argparse
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / "crates/vearth-app/assets/materials"
SIDE = 128
SAMPLES = 4
AUTHOR_SIDE = SIDE * SAMPLES
NAMES = ("brick", "stone", "plaster", "timber", "tile", "slate", "panel", "metal",
         "siding", "shingle", "flat", "thatch", "namako", "limestone", "marble", "earthen",
         "corrugated", "plain-tile", "rubble", "painted-boards", "wood-shingle", "woven-panel",
         "cement-sheet", "turf", "azulejo", "azulejo-pattern")
# Preview swatches only: the game owns regional RGB, OSM overrides and patina.
COLORS = ((154, 87, 65), (158, 151, 131), (222, 211, 186), (104, 76, 51),
          (173, 99, 65), (79, 91, 104), (181, 181, 172), (87, 125, 117),
          (174, 175, 160), (88, 83, 78), (153, 151, 138), (163, 142, 95),
          (83, 91, 100), (194, 186, 157), (216, 212, 200), (175, 132, 94),
          (117, 139, 143), (153, 76, 53), (158, 151, 131), (147, 47, 39), (124, 108, 82), (190, 184, 150),
          (130, 132, 125), (98, 120, 59), (41, 135, 148), (32, 78, 149))


def smooth(value):
    t = np.clip(value, 0, 1)
    return t * t * (3 - 2 * t)


def blur(field, radius):
    f = np.fft.fftfreq(field.shape[0])
    kernel = np.exp(-2 * np.pi**2 * radius**2 * (f[:, None]**2 + f[None, :]**2))
    return np.fft.ifft2(np.fft.fft2(field) * kernel).real


def noise(rng, radius):
    field = blur(rng.uniform(-1, 1, (AUTHOR_SIDE, AUTHOR_SIDE)), radius * SAMPLES)
    return np.tanh((field - field.mean()) / field.std() * 0.65)


def downsample(field):
    return field.reshape(SIDE, SAMPLES, SIDE, SAMPLES).mean(axis=(1, 3))


def strokes(rng, x, y, count, length, width):
    """Soft overlapping trowel passes, with periodically wrapped brush ends."""
    result = np.zeros_like(x)
    for _ in range(count):
        cx, cy = rng.random(2)
        angle = rng.uniform(-0.7, 0.7)
        dx, dy = (x - cx + 0.5) % 1 - 0.5, (y - cy + 0.5) % 1 - 0.5
        u = (dx * np.cos(angle) + dy * np.sin(angle)) / length
        v = (-dx * np.sin(angle) + dy * np.cos(angle)) / width
        mask = np.exp(-2 * (u**4 + v**2))
        result += mask * rng.uniform(-1, 1)
    return np.tanh(result)


def masonry(rng, x, y, rows, columns, rough, jitter, stagger=True):
    """Bonded blocks retain level courses; handmade edges vary inside the joint."""
    heights = rng.uniform(1 - jitter, 1 + jitter, rows)
    row_edges = np.r_[0, np.cumsum(heights / heights.sum())]
    face, piece, facet = (np.zeros_like(x) for _ in range(3))
    for row in range(rows):
        bottom, top = row_edges[row:row + 2]
        vertical = np.minimum(y - bottom, top - y)
        widths = rng.uniform(1 - jitter, 1 + jitter, columns)
        edges = np.r_[0, np.cumsum(widths / widths.sum())]
        shift = (row % 2 if stagger else 0) / columns * 0.5 + 0.031
        sx = (x + shift) % 1
        for column in range(columns):
            left, right = edges[column:column + 2]
            inside = (vertical >= 0) & (sx >= left) & (sx < right)
            edge = np.minimum(vertical, np.minimum(sx - left, right - sx))
            # 4 m repeat: 3–8 mm irregularities and 12–24 mm bevel shoulders.
            chipped = edge - np.maximum(rough, 0) * (0.002 if jitter else 0.0005)
            coverage = smooth((chipped - 0.0015) / (0.005 if jitter else 0.003))
            face += np.where(inside, coverage, 0)
            pigment = rng.uniform(-1, 1)
            piece += np.where(inside, pigment * coverage, 0)
            tilt = (sx - (left + right) * 0.5) / (right - left)
            facet += np.where(inside, (tilt * rng.uniform(-1, 1)) * coverage, 0)
    return face, piece, facet


def author(name, index):
    rng = np.random.Generator(np.random.PCG64(20260915 + index * 101))
    y, x = (np.mgrid[:AUTHOR_SIDE, :AUTHOR_SIDE] + 0.5) / AUTHOR_SIDE
    broad, middle, small = (noise(rng, r) for r in (15, 4, 1.2))
    pores = np.maximum(small - 0.55, 0)
    if name in ("brick", "stone", "limestone"):
        rows, columns, jitter = {"brick": (28, 14, 0.10), "stone": (10, 6, 0.35),
                                 "limestone": (8, 5, 0.20)}[name]
        face, piece, facet = masonry(rng, x, y, rows, columns, middle, jitter)
        joint = 1 - face
        split = strokes(rng, x, y, 30, 0.10, 0.013) if name != "brick" else middle
        tone = piece * 0.12 + broad * 0.025 + split * face * 0.026 - joint * 0.105
        height = face * 0.014 + facet * 0.004 + split * face * 0.002 - pores * face * 0.003
        roughness = 0.87 + joint * 0.065 + small * 0.015 - piece * 0.014
    elif name in ("plaster", "earthen"):
        hand = strokes(rng, x, y, 65, 0.13, 0.037)
        earth = name == "earthen"
        tone = broad * (0.043 if earth else 0.042) + hand * 0.028 + middle * 0.009 - pores * 0.012
        height = broad * (0.006 if earth else 0.002) + hand * (0.008 if earth else 0.003) + middle * 0.0015 - pores * 0.001
        roughness = (0.95 if earth else 0.91) + hand * 0.017 + small * 0.013
    elif name == "timber":
        columns = 16
        board_x = (x + 0.017 + middle * 0.0008) * columns
        column = np.floor(board_x).astype(int) % columns
        bx = board_x % 1
        edge = np.minimum(bx, 1 - bx) / columns
        face = smooth((edge - 0.0008) / 0.003)
        values = rng.uniform(-1, 1, columns)[column]
        # Grain bends around restrained, elongated knots instead of isotropic noise.
        fibers = np.sin(2 * np.pi * (x * 76 + broad * 0.34 + np.sin(y * 2 * np.pi) * 0.35))
        knot = np.zeros_like(x)
        for board in (1, 5, 9, 14):
            cx, cy = (board + 0.5) / columns - 0.017, rng.random()
            dx, dy = (x - cx + 0.5) % 1 - 0.5, (y - cy + 0.5) % 1 - 0.5
            radius = np.hypot(dx / 0.016, dy / 0.045)
            knot += np.exp(-radius * radius * 1.5) + np.sin(radius * 10) * np.exp(-radius * radius) * 0.25
        tone = values * face * 0.070 + broad * 0.023 + fibers * face * 0.018 - knot * 0.075 - (1 - face) * 0.11
        height = face * 0.008 + fibers * face * 0.0012 + broad * 0.0015 - knot * 0.001
        roughness = 0.84 + small * 0.016 - values * 0.025 + (1 - face) * 0.06
    elif name in ("tile", "slate", "shingle"):
        columns, rows = {"tile": (14, 12), "slate": (12, 16), "shingle": (12, 24)}[name]
        row = np.floor(y * rows).astype(int)
        sx = (x * columns + (row % 2) * 0.5 + 0.17)
        column, bx, by = np.floor(sx).astype(int) % columns, sx % 1, (y * rows) % 1
        pieces = rng.uniform(-1, 1, (rows, columns))[row, column]
        # The upper overlap is quiet; the exposed lower edge carries the lip.
        side = smooth((np.minimum(bx, 1 - bx) - np.maximum(middle, 0) * 0.025) / 0.065)
        course = smooth(by / 0.075) * smooth((1 - by) / 0.14)
        face = side * course
        if name == "tile":
            crown = np.sin(np.pi * bx)**0.8
            firing = broad * 0.040 + middle * 0.012
            tone = pieces * face * 0.150 + firing * face - (1 - course) * 0.078 - (1 - side) * 0.048
            height = crown * course * 0.017 + middle * face * 0.001
            roughness = 0.67 + pieces * 0.035 + small * 0.012 + (1 - face) * 0.08
        elif name == "slate":
            cleft = strokes(rng, x, y, 42, 0.06, 0.007)
            tone = pieces * face * 0.080 + cleft * face * 0.055 + broad * 0.020 - (1 - face) * 0.090
            height = face * (0.005 + by * 0.010 + cleft * 0.0025)
            roughness = 0.77 + cleft * 0.042 + pieces * 0.020 + (1 - face) * 0.07
        else:
            tone = pieces * face * 0.060 + middle * 0.022 + small * 0.012 - (1 - face) * 0.085
            height = face * (0.003 + by * 0.004) + small * face * 0.0008
            roughness = 0.93 + small * 0.018 - pieces * 0.012
    elif name == "corrugated":
        # 52 ripples over 4 m approximates a 76 mm pitch / 16 mm rib profile.
        ripple = np.cos(2 * np.pi * (x * 52 + 0.17))
        sheet = rng.uniform(-1, 1, 5)[np.floor(x * 5).astype(int)]
        tone = broad * 0.017 + sheet * 0.018 + middle * 0.007
        height = ripple * 0.008 + broad * 0.0005
        roughness = 0.51 + broad * 0.050 + middle * 0.018
    elif name == "cement-sheet":
        # Broad mineral-sheet corrugations: roughly 143 mm pitch. Relief is
        # deliberately shallower than the 47.6 mm product profile to fit the
        # shared normal signal; neither metallic sheen nor slate joints apply.
        ripple = np.cos(2 * np.pi * (x * 28 + 0.17))
        sheet = rng.uniform(-1, 1, 4)[np.floor(x * 4).astype(int)]
        lap = np.exp(-(np.sin(2 * np.pi * y) / 0.055)**2)
        tone = sheet * 0.038 + broad * 0.030 + middle * 0.014 - lap * 0.025
        height = ripple * 0.008 + broad * 0.0008 - lap * 0.001
        roughness = 0.90 + broad * 0.035 + small * 0.018
    elif name == "turf":
        # A continuous sod surface with soft clumps and matted grass. This
        # has no tile courses or bundled-thatch lines and adds no grass mesh.
        clumps = strokes(rng, x, y, 95, 0.038, 0.015)
        fibers = np.sin(2 * np.pi * (x * 113 + y * 37 + broad * 0.4))
        tone = broad * 0.065 + clumps * 0.031 + middle * 0.021 + fibers * 0.004
        height = broad * 0.012 + clumps * 0.007 + middle * 0.0015
        roughness = 0.94 + broad * 0.022 + small * 0.018
    elif name == "plain-tile":
        # 160 mm face width / 100 mm course exposure: flat overlapping clay.
        row = np.floor(y * 40).astype(int)
        sx = x * 25 + (row % 2) * 0.5 + 0.17
        column, bx, by = np.floor(sx).astype(int) % 25, sx % 1, (y * 40) % 1
        piece = rng.uniform(-1, 1, (40, 25))[row, column]
        side = smooth(np.minimum(bx, 1 - bx) / 0.060)
        lip = smooth(by / 0.13) * smooth((1 - by) / 0.12)
        face = side * lip
        tone = piece * face * 0.11 + broad * 0.030 - (1 - face) * 0.075
        height = face * (0.003 + by * 0.008) + middle * face * 0.0005
        roughness = 0.72 + piece * 0.028 + small * 0.017 + (1 - face) * 0.07
    elif name == "rubble":
        # A periodic irregular polygon field gives uncoursed angular stones;
        # unlike ashlar, no common horizontal bed joint crosses the whole tile.
        nearest = np.full_like(x, 100.0)
        second = nearest.copy()
        piece = np.zeros_like(x)
        tilt = np.zeros_like(x)
        for cx, cy, pigment, slope in rng.uniform(0, 1, (65, 4)):
            dx, dy = (x - cx + 0.5) % 1 - 0.5, (y - cy + 0.5) % 1 - 0.5
            distance = np.hypot(dx, dy * 1.25)
            closer = distance < nearest
            second = np.where(closer, nearest, np.minimum(second, distance))
            nearest = np.minimum(nearest, distance)
            piece = np.where(closer, pigment * 2 - 1, piece)
            tilt = np.where(closer, dx * (slope * 2 - 1), tilt)
        gap = (second - nearest) * 0.5 - np.maximum(middle, 0) * 0.001
        face = smooth((gap - 0.0015) / 0.0045)
        split = strokes(rng, x, y, 45, 0.055, 0.011)
        tone = piece * face * 0.11 + split * face * 0.026 + broad * 0.020 - (1 - face) * 0.08
        height = face * (0.018 + split * 0.005 + tilt * 0.022) - pores * face * 0.002
        roughness = 0.88 + split * 0.023 + (1 - face) * 0.06
    elif name == "painted-boards":
        # Opaque paint suppresses knots and heartwood pigments while retaining
        # the vertical tongue-and-groove seams and a restrained brush relief.
        sx = x * 20 + 0.17
        board, bx = np.floor(sx).astype(int) % 20, sx % 1
        face = smooth(np.minimum(bx, 1 - bx) / 0.065)
        paint = rng.uniform(-1, 1, 20)[board]
        brush = np.sin(2 * np.pi * (x * 75 + broad * 0.12))
        tone = paint * 0.018 + broad * 0.013 + brush * 0.003 - (1 - face) * 0.075
        height = face * 0.004 + brush * face * 0.00035 + middle * 0.00045
        roughness = 0.73 + broad * 0.031 + paint * 0.013 + (1 - face) * 0.07
    elif name == "woven-panel":
        # Alternating diagonal organic ribbons, about 70 mm apart. This is a
        # broad palm-panel approximation, not a measured species-specific weave.
        u, v = (x + y) * 40 + 0.17, (x - y) * 40 + 0.31
        a, b = np.sin(np.pi * u), np.sin(np.pi * v)
        crossing = a * b
        overlap = np.tanh(crossing * 4)
        strand = (np.cos(2 * np.pi * u) + np.cos(2 * np.pi * v)) * 0.5
        tone = overlap * 0.038 + strand * 0.016 + broad * 0.038 + middle * 0.014
        height = crossing * 0.0025 + strand * 0.0006 + broad * 0.0006
        roughness = 0.91 + broad * 0.029 + overlap * 0.018
    elif name == "wood-shingle":
        rows, columns = 26, 23
        face, pigment, grain = (np.zeros_like(x) for _ in range(3))
        height = np.zeros_like(x)
        for row in range(rows):
            widths = rng.uniform(0.55, 1.45, columns)
            edges = np.r_[0, np.cumsum(widths / widths.sum())]
            sx = (x + (row % 2) * 0.019 + 0.011) % 1
            by = y * rows - row
            for col in range(columns):
                bx = (sx - edges[col]) / (edges[col + 1] - edges[col])
                inside = (by >= 0) & (by < 1) & (bx >= 0) & (bx < 1)
                side = smooth(np.minimum(bx, 1 - bx) / 0.05)
                lip = smooth(by / 0.10) * smooth((1 - by) / 0.15)
                piece = np.where(inside, side * lip, 0)
                fibers = np.sin(2 * np.pi * (bx * rng.uniform(3, 6) + middle * 0.04))
                face += piece
                pigment += piece * rng.uniform(-1, 1)
                grain += fibers * piece
                height += piece * (0.003 + by * 0.006 + fibers * 0.00065)
        tone = pigment * 0.09 + grain * 0.020 + broad * 0.027 - (1 - face) * 0.085
        roughness = 0.86 + pigment * 0.025 + grain * 0.014 + (1 - face) * 0.07
    elif name == "panel":
        face, piece, _ = masonry(rng, x, y, 3, 3, middle, 0, stagger=False)
        tone = broad * 0.013 + middle * 0.006 + piece * 0.016 - (1 - face) * 0.10
        height = broad * 0.0015 - (1 - face) * 0.004
        roughness = 0.76 + broad * 0.023 + (1 - face) * 0.09
    elif name == "metal":
        sx = (x * 8 + 0.17) % 1
        seam = np.exp(-(np.minimum(sx, 1 - sx) / 0.030)**2)
        sheet = rng.uniform(-1, 1, 8)[np.floor(x * 8 + 0.17).astype(int) % 8]
        hammered = strokes(rng, x, y, 34, 0.018, 0.07)
        tone = sheet * 0.022 + broad * 0.028 + hammered * 0.025 - seam * 0.045
        height = seam * 0.016 + hammered * 0.002 + broad * 0.001
        roughness = 0.49 + broad * 0.065 + hammered * 0.035 + seam * 0.04
    elif name == "siding":
        by = (y * 20 + 0.19) % 1
        row = np.floor(y * 20 + 0.19).astype(int) % 20
        paint = rng.uniform(-1, 1, 20)[row]
        lip = smooth(by / 0.11)
        brush = np.sin(2 * np.pi * (y * 54 + broad * 0.2))
        tone = paint * 0.025 + broad * 0.019 + brush * 0.005 - (1 - lip) * 0.095
        height = lip * (1 - by) * 0.010 + middle * 0.001
        roughness = 0.84 + middle * 0.020 + paint * 0.015
    elif name == "flat":
        aggregate = np.maximum(small - 0.10, 0)
        tone = broad * 0.029 + middle * 0.015 + aggregate * 0.014
        height = broad * 0.002 + middle * 0.001 + aggregate * 0.0012
        roughness = 0.94 + middle * 0.013 + aggregate * 0.018
    elif name == "thatch":
        # Unequal sheaves and wavy reed groups read as a surface, not a sine grating.
        fibers = np.sin(2 * np.pi * (x * 57 + broad * 0.38 + middle * 0.11))
        bundles = np.sin(2 * np.pi * (x * 10 + broad * 0.15))
        ends = strokes(rng, x, y, 70, 0.021, 0.09)
        tone = broad * 0.025 + bundles * 0.021 + fibers * 0.017 + ends * 0.030
        height = bundles * 0.003 + fibers * 0.0017 + ends * 0.0018
        roughness = 0.95 + small * 0.018
    elif name in ("azulejo", "azulejo-pattern"):
        # 28 squares per four metres approximate the documented 13–15 cm
        # Portuguese façade tiles. Integrate narrow grout before atlas reduction.
        tx, ty = x * 28 + 0.19, y * 28 + 0.27
        bx, by = tx % 1, ty % 1
        distance = np.minimum(np.minimum(bx, 1 - bx), np.minimum(by, 1 - by)) * 4 / 28
        joint = np.clip((0.001 + 2 / AUTHOR_SIDE - distance) * AUTHOR_SIDE / 4, 0, 1)
        tile_noise = rng.uniform(-1, 1, (28, 28))[np.floor(ty).astype(int) % 28,
                                                          np.floor(tx).astype(int) % 28]
        face = 1 - joint
        dome = np.sin(np.pi * bx)**2 * np.sin(np.pi * by)**2
        height = dome * 0.00055 - joint * 0.00065 + middle * 0.00010
        roughness = 0.35 + tile_noise * 0.027 + broad * 0.016 + joint * 0.30
        if name == "azulejo":
            tone = tile_noise * 0.034 + broad * 0.009 - joint * 0.07
        else:
            # Original two-by-two geometric repeat, not a copied historic panel.
            # R is linear ink coverage; relief follows the ceramic, not the ink.
            u, v = (tx / 2) % 1 - 0.5, (ty / 2) % 1 - 0.5
            ring = 1 - smooth((abs(np.sqrt(u*u + v*v) - 0.31) - 0.025) / 0.035)
            petals = np.maximum(np.exp(-((u / 0.36)**2 + (v / 0.105)**2)**3),
                                np.exp(-((v / 0.36)**2 + (u / 0.105)**2)**3))
            diamond = 1 - smooth((abs(abs(u) + abs(v) - 0.49) - 0.014) / 0.027)
            ink = np.maximum(np.maximum(ring, petals * 0.85), diamond * 0.8)
            tone = 0.025 + ink * face * (0.93 + tile_noise * 0.015)
    elif name == "namako":
        a, b = ((x + y) * 8 + 0.17) % 1, ((x - y) * 8 + 0.17) % 1
        distance = np.minimum(np.minimum(a, 1 - a), np.minimum(b, 1 - b))
        joint = 1 - smooth((distance + middle * 0.005) / 0.105)
        tone = joint * 0.32 + broad * 0.016 + middle * 0.009
        height = joint * 0.011 + middle * (1 - joint) * 0.0007
        roughness = 0.72 + joint * 0.20 + middle * 0.023
    elif name == "marble":
        # Two connected mineral scales; veins remain pigment, not carved channels.
        warp = broad * 0.21 + middle * 0.024
        primary = abs(np.sin(2 * np.pi * (x + y * 2 + warp)))
        vein = np.exp(-(primary / 0.065)**2)
        branch = np.exp(-(abs(np.sin(2 * np.pi * (x * 3 - y + warp * 2))) / 0.045)**2)
        tone = broad * 0.023 + middle * 0.010 - vein * 0.12 - branch * vein**0.2 * 0.048
        height = broad * 0.0015 + middle * 0.0005 - vein * 0.0006
        roughness = 0.56 + broad * 0.024 + vein * 0.055
    else:
        raise ValueError(name)
    # These shallow artistic joints must fit the shared signed-slope range;
    # keep the measured corrugated-sheet profile at its authored 16 mm height.
    height *= {"brick": 0.88, "stone": 0.92, "limestone": 0.92,
               "tile": 0.88, "rubble": 0.68}.get(name, 1.0)
    # Differentiate before integration: narrow ribs lose most of their slope
    # if central differences are taken after reducing to the atlas resolution.
    return downsample(tone), height, downsample(roughness)


def encode(fields, coverage=False):
    tone, height, roughness = fields
    gradients = [downsample((np.roll(height, -1, axis) - np.roll(height, 1, axis))
                            * AUTHOR_SIDE / 8) for axis in (1, 0)]
    linear = np.stack((tone if coverage else 0.5 + tone - tone.mean(), 0.5 + gradients[0],
                       0.5 + gradients[1], roughness), axis=-1)
    assert np.isfinite(linear).all() and linear.min() > 0 and linear.max() < 1, "clipped material signal"
    return np.floor(linear * 255 + 0.5).astype(np.uint8)


def validate_slope_filter():
    x = (np.arange(AUTHOR_SIDE) + 0.5) / AUTHOR_SIDE
    height = np.broadcast_to(0.008 * np.cos(2 * np.pi * 52 * x),
                             (AUTHOR_SIDE, AUTHOR_SIDE))
    gradient = downsample((np.roll(height, -1, 1) - np.roll(height, 1, 1)) * AUTHOR_SIDE / 8)
    # Integrate the analytic physical derivative over each atlas texel.
    boundaries = np.arange(SIDE + 1) / SIDE
    expected = np.diff(0.008 * np.cos(2 * np.pi * 52 * boundaries)) * SIDE / 4
    assert np.sqrt(np.mean((gradient[0] - expected)**2)) < 0.018
    assert np.max(abs(gradient)) > 0.45, "76 mm corrugations lost their physical slope"


def validate(name, fields, tile):
    if name == "azulejo-pattern":
        assert abs(tile[:, :, 0].mean() / 255 - fields[0].mean()) < 0.5 / 255
        assert 0.20 < fields[0].mean() < 0.65, "pattern has lost its white ground or ink"
    else:
        assert abs(tile[:, :, 0].mean() - 127.5) < 0.02, f"{name}: regional tint drift"
    assert tile[:, :, 0].std() > 2, f"{name}: no readable tone structure"
    assert tile[:, :, 3].std() > 1, f"{name}: uniform roughness"
    height = fields[1]
    gx, gy = [(np.roll(height, -1, axis) - np.roll(height, 1, axis))
              * AUTHOR_SIDE / 8 for axis in (1, 0)]
    curl = np.roll(gx, -1, 0) - np.roll(gx, 1, 0) - np.roll(gy, -1, 1) + np.roll(gy, 1, 1)
    assert abs(curl).max() < 1e-12, f"{name}: nonintegrable authored relief"
    decoded = tile[:, :, 1:3].astype(float) / 255 - 0.5
    expected = np.stack((downsample(gx), downsample(gy)), axis=-1)
    assert abs(decoded - expected).max() <= 0.5 / 255 + 1e-12, f"{name}: filtered slope mismatch"
    # Joints may meet the tile boundary. A narrow joint exactly aligned to
    # a texel edge can concentrate twice the jump of the same interior joint
    # split across two texels; compare with the strongest interior cut.
    for axis in (0, 1):
        delta = abs(tile.astype(float) - np.roll(tile.astype(float), 1, axis))
        cuts = delta.mean(axis=1 - axis)
        assert (cuts[0] <= cuts[1:].max(axis=0) * 2 + 1).all(), (name, axis, cuts[0])
    assert abs(np.corrcoef(fields[0].flat, downsample(fields[1]).flat)[0, 1]) < 0.9999, f"{name}: relief duplicates pigment"


def preview(directory, fields, tiles):
    directory.mkdir(parents=True, exist_ok=True)
    sheet = Image.new("RGB", (4 * 300, ((len(NAMES) + 3) // 4) * 330), (27, 32, 35))
    draw = ImageDraw.Draw(sheet)
    for index, (name, field, tile, color) in enumerate(zip(NAMES, fields, tiles, COLORS)):
        tone = (tile[:, :, 0].astype(float) / 255 - 0.5) * 1.6
        pigment = np.asarray(color) / 255
        pigment = np.where(pigment <= 0.04045, pigment / 12.92, ((pigment + 0.055) / 1.055)**2.4)
        gain = 1 + np.minimum(tone, 0)[:, :, None] * (0.68, 0.58, 0.42) + np.maximum(tone, 0)[:, :, None] * (0.55, 0.42, 0.20)
        dx, dy = (tile[:, :, 1:3].astype(float) / 255 - 0.5).transpose(2, 0, 1)
        lighting = (0.82 - dx * 0.7 - dy * 0.5) / np.sqrt(1 + dx**2 + dy**2)
        if name == "azulejo-pattern":
            ground = np.array((0.806952258, 0.822785754, 0.752942217))
            coverage = tile[:, :, 0:1].astype(float) / 255
            pigment = ground * (1 - coverage) + pigment * coverage
            gain = 1.0
        linear = np.clip(pigment * gain * lighting[:, :, None], 0, 1)
        rgb = np.where(linear <= 0.0031308, linear * 12.92, 1.055 * linear**(1 / 2.4) - 0.055)
        image = Image.fromarray(np.floor(rgb * 255 + 0.5).astype(np.uint8))
        image.save(directory / f"{name}.png")
        for channel, values in zip(("tone", "height", "roughness"), field):
            values = (0.5 + values - values.mean()) if channel == "tone" and name != "azulejo-pattern" else values
            if channel == "height":
                values = downsample(values)
                values = (values - values.min()) / np.ptp(values)
            Image.fromarray(np.floor(values * 255 + 0.5).astype(np.uint8)).save(directory / f"{name}-{channel}.png")
        left, top = index % 4 * 300 + 20, index // 4 * 330 + 34
        tiled = Image.fromarray(np.tile(np.asarray(image), (2, 2, 1)))
        sheet.paste(tiled, (left, top))
        draw.text((left, top - 22), name.upper(), fill=(237, 230, 215))
        span = 16 if name == "marble" else 8
        draw.text((left, top + 266), f"{span} m span / illustrative lighting", fill=(168, 178, 177))
    sheet.save(directory / "construction-contact-sheet.png")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="verify exact baked data and generator provenance")
    parser.add_argument("--preview", type=Path, help="write channel maps and a lit two-repeat contact sheet")
    args = parser.parse_args()
    validate_slope_filter()
    fields = [author(name, index) for index, name in enumerate(NAMES)]
    tiles = [encode(field, name == "azulejo-pattern") for name, field in zip(NAMES, fields)]
    for name, field, tile in zip(NAMES, fields, tiles):
        validate(name, field, tile)
    data = b"".join(tile.tobytes() for tile in tiles)
    manifest = {
        "schema": 1, "asset": "construction-tiles.rgba8", "layers": dict(zip(NAMES, (*range(23, 39), *range(45, 45 + len(NAMES) - 16)))),
        "dimensions": [SIDE, SIDE], "bytes": len(data), "authoring_samples_per_texel": SAMPLES**2,
        "period_m": {"construction": 4, "marble": 8, "earthen": 4}, "encoding": "linear R=0.5+zero-mean tone; GB=0.5+central dH/dUV/4; A=perceptual roughness",
        "channel_overrides": {"azulejo-pattern": "R=linear ink coverage; source RGB is ink over sRGB(232,234,225) ceramic, not whole-façade average"},
        "bake": "python3 tools/bake-construction-materials.py",
        "sources": {str(Path(__file__).relative_to(ROOT)): hashlib.sha256(Path(__file__).read_bytes()).hexdigest()},
        "sha256": hashlib.sha256(data).hexdigest(),
        "provenance": "Original deterministic illustrative art; no third-party texture pixels. Metric height and perceptual roughness are artistic, unmeasured signals. Regional pigment remains supplied by the game.",
        "physical_references": [
            {"url": "https://visitporto.travel/storage/01HRWD2GGEDZY75FC3ZZW3TAYR.pdf", "application": "Official Porto guide page10 shows ordinary plain turquoise glazed façades and blue-on-white repeated tile patterns; keep two pigment semantics distinct. Original geometric art, no source pixels."},
            {"url": "https://hemerotecadigital.cm-lisboa.pt/obras/LisboaRevM/N5_6/N5_6_master/N5_6.pdf", "application": "Municipal historical factory inventory records13,14 and15 cm square tiles. The143 mm repeat,2 mm understated grout and glaze roughness are illustrative approximations, not measured product replication."},
            {"url": "https://media.eternit.co.uk/doc_216586_gb/original/-448648132/profile-6-choosing-and-using-guide.pdf?brand=eterniteuronit&market=gb", "application": "Mineral corrugated roof sheets have wider ribs than the small-wave metal sheet. The 146.5 mm reference pitch is approximated by a periodic 143 mm pitch; relief is artistically reduced, not a measured product reproduction."},
            {"url": "https://visitfaroeislands.com/en/see-do/cultural-experiences/art-and-architecture/architecture", "application": "Living turf roofing is a continuous grass-covered surface, distinct from straw thatch and ceramic courses. Original neutral sod texture; color comes from the local roof recipe."},
            {"url": "https://www.undp.org/pacific/blog/preserving-mangroves-angouru-community-led-effort-combat-climate-change", "application": "Actual Angouru organic woven wall panels, sheet and thatch roofs. The woven pattern is original and broadly scaled, not a claim to a measured species-specific weave."},
            {"url": "https://www.inbar.int/wp-content/uploads/2020/05/1489455390.pdf", "application": "Crossed thin organic strips form woven panels; distinguish exposed matting from plastered bamboo substrate."},
            {"url": "https://sikumbang.tapera.go.id/lokasi-perumahan/ATB0420012020T001", "application": "Actual built-house record specifies coloured tile-profile metal, represented by metal variant 3 sharing the rounded tile layer."},
            {"url": "https://lysaght.com/profiles/custom-orb", "application": "Corrugated sheet ripple geometry, approximately 76 mm pitch and 16 mm height; standing seam remains a separate finish."},
            {"url": "https://www.wienerberger.co.uk/product-range/roof/plain-tile-brown.html", "application": "Plain clay tile dimensions and 100 mm exposure; distinct flat overlapping profile."},
            {"url": "https://historicengland.org.uk/advice/your-home/maintain-repair/walls/", "application": "Exposed rubble and ashlar are distinct finishes; hidden masonry must not replace limewash or render."},
            {"url": "https://trada.co.uk/start-here/timber-cladding/", "application": "Vertical timber cladding remains distinct from horizontal clapboard; painted board relief is original illustrative art."},
            {"url": "https://www.cedarbureau.org/product-types/certigrade-shingles/", "application": "Wood shingles have variable widths, taper and longitudinal edge grain; composition shingles remain separate."}
        ],
        "art_references": [
            {"url": "https://www.adobe.com/products/substance3d/magazine/the-creation-of-bell-island.html", "application": "Shape hierarchy, selectively placed coarse defects and restrained noise; original material generators."},
            {"url": "https://help.steampowered.com/en/faqs/view/0688-7692-4D5A-1935", "application": "Value restraint, quiet regions and in-game readability; original neutral tint-preserving tone maps."},
            {"url": "https://www.gdcvault.com/play/1024562", "application": "Nintendo visual-direction reference, not a claim to reproduce proprietary Zelda texture maps."},
            {"url": "https://www.gdcvault.com/play/1025015/Visual-Adventures-on-Sea-of", "application": "Rare art-direction reference for consistent stylized visual rules across materials."}
        ]
    }
    outputs = {ASSETS / "construction-tiles.rgba8": data,
               ASSETS / "construction-tiles.source.json": (json.dumps(manifest, indent=2) + "\n").encode()}
    for path, content in outputs.items():
        if args.check:
            assert path.read_bytes() == content, f"stale construction bake: {path}"
        else:
            path.write_bytes(content)
    if args.preview:
        preview(args.preview, fields, tiles)
    print(f"{'Verified' if args.check else 'Baked'} {len(tiles)} construction layers; {len(data):,} bytes")


if __name__ == "__main__":
    main()
