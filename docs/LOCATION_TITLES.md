# Arrival titles

`location_title/catalog.rs` contains 110 authored profiles: 97 bounded local
profiles and 13 broader fallbacks. Unknown geography uses the neutral world
profile. Selection uses the bundled administrative polygon's country and the
resolved query coordinate; it never guesses geography from the displayed name.
Local radii constrain art direction, not borders or labels. Overlaps choose the
smallest normalized distance. Country guards use the atlas's exact source names.

Names still come from the existing location resolver. The map keeps its combined
English-preferred label. Arrival titles separate primary and region; native names
must belong to that same selected primary, match the profile's language/script,
and pass complete font cmap coverage. Plain `name` can be used as a source-local
fallback. Provider `pgf:name` glyph encodings are excluded. Discarding a primary
also discards its alternate names. No hand-authored translations enter gameplay.

The pinned egui supports LTR shaping, including Indic clusters, but lacks reliable
RTL paragraph layout. Arabic/Hebrew use the English-preferred name or the available
region fallback. Unsupported scripts fall back atomically; no missing-glyph
mixture is presented as a native title. Native-script runs have no added tracking.

Each title freezes its profile and source names at acceptance. Only opacity
changes during the reveal; positions, font size, tracking, line breaks, extrusion,
and outlines remain fixed. Layout adapts to the viewport, measures before wrapping,
and limits exceptionally long titles to two rows. Existing confirmation, spawn,
high-altitude suppression, and retry behavior remain in the location tracker.
There are no title panels or decorative lines above the text.

## Art-direction references

These catalogs informed loose choices of contrast, rhythm, weight, print depth,
and color. No poster artwork, proprietary lettering, or cultural symbols are copied.
Typography is contemporary art direction, not a claim that one design represents
every community in a region.

- [Letterform Archive: Indian Movie Posters](https://letterformarchive.org/news/indian-movie-posters/): varied Indic scripts, bold film lettering, layered ink.
- [Letterform Archive: The Complete Commercial Artist](https://letterformarchive.org/news/the-complete-commercial-artist/): contrast between Japanese display lettering and quieter book typography.
- [Museum für Gestaltung: Poster Collection](https://museum-gestaltung.ch/en/poster): Swiss editorial discipline and variation across travel and cultural posters.
- [National Library of Australia: Aldo Cosomati travel poster](https://catalogue.nla.gov.au/catalog/7426718): spacious travel-poster hierarchy.
- [PintassilgoPrints: O Grande Kilapy](https://pintassilgoprints.com/fonts-in-use/swung-note-o-grande-kilapy/): expressive title lettering in an Angola/Brazil/Portugal film production.

## Verification

```sh
python3 tools/verify-title-fonts.py
cargo test -p vearth-data --all-features location_ --lib
cargo test -p vearth-app --all-features location --lib
cargo run -p vearth-app --example title-catalog -- output/title-catalog
```

The example renders all profiles with production fonts, shaping, layout, and
wgpu painting, then writes PNGs and an HTML contact sheet. Its sample names are
explicit proof fixtures. Append a profile key and a current engine screenshot
path to inspect that treatment over rendered terrain, for example:

```sh
cargo run -p vearth-app --example title-catalog -- output/title-city new-york output/typography-review/manhattan-current.png
```

Fonts are full upstream files with pinned revisions and hashes in
`assets/fonts/titles/manifest.json`; `tools/verify-title-fonts.py --fetch` restores
them. Dedicated arrival families leave the rest of the UI's fonts unchanged.
