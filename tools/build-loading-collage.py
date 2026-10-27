#!/usr/bin/env python3
"""Compose the browser app's loading collage from the website gallery.

Tiles the gallery's 2048-pixel captures on a 6 x 6 grid of 16:9 cells, three
of them as 2 x 2 features, and writes one WebP:

    python3 tools/build-loading-collage.py [--width 2560] [--quality 55]
"""

import argparse
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
GALLERY = ROOT / "site/assets/web"
OUTPUT = ROOT / "web/loading-collage.webp"
COLUMNS, ROWS = 6, 6
GAP = 3
BACKGROUND = (11, 14, 16)
# (column, row) of each 2 x 2 feature's top-left cell.
FEATURES = {
    "tromso-aurora": (0, 0),
    "matterhorn-winter": (4, 1),
    "manhattan-sunset": (1, 4),
}
# Single cells, filled in reading order around the features.
SINGLES = [
    "dubrovnik-morning", "rio-sunset", "tokyo-dusk", "machu-picchu-clouds",
    "monument-valley", "lake-como", "cesky-krumlov-snow", "yosemite-evening",
    "paris-etoile-topdown", "fuji-dawn", "venice-lagoon-sunset", "grand-canyon-sun",
    "lofoten-evening", "dubai-marina-sunset", "hallstatt-close", "amsterdam-canals-topdown",
    "himalaya-annapurna", "san-francisco-sunset", "chefchaouen-morning", "palmanova-star",
    "istanbul-old-city", "tre-cime-sunset", "london-thames-sunset", "seattle-rainier",
]


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--width", type=int, default=2560)
    parser.add_argument("--quality", type=int, default=55)
    args = parser.parse_args()
    cell_w = args.width // COLUMNS
    cell_h = cell_w * 9 // 16
    canvas = Image.new("RGB", (cell_w * COLUMNS, cell_h * ROWS), BACKGROUND)

    def place(name, column, row, span):
        width, height = cell_w * span - GAP, cell_h * span - GAP
        image = Image.open(GALLERY / f"{name}-2048.webp").convert("RGB")
        canvas.paste(image.resize((width, height), Image.LANCZOS), (column * cell_w, row * cell_h))

    taken = set()
    for name, (column, row) in FEATURES.items():
        place(name, column, row, 2)
        taken |= {(column + dx, row + dy) for dx in (0, 1) for dy in (0, 1)}
    free = [(c, r) for r in range(ROWS) for c in range(COLUMNS) if (c, r) not in taken]
    if len(free) != len(SINGLES):
        raise SystemExit(f"{len(free)} free cells for {len(SINGLES)} single images")
    for name, (column, row) in zip(SINGLES, free):
        place(name, column, row, 1)
    canvas.save(OUTPUT, "WEBP", quality=args.quality, method=6)
    print(f"wrote {OUTPUT.relative_to(ROOT)}: {canvas.size[0]} x {canvas.size[1]}, "
          f"{OUTPUT.stat().st_size // 1024} KB")


if __name__ == "__main__":
    main()
