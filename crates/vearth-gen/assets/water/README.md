# Distinctive water catalog

`catalog.json` authors the optical palettes and identifies the pinned source
geometry. `sources/*.json.gz` retain the OSM API responses used by the generator;
their uncompressed SHA-256 hashes and feature versions must match the catalog.

```sh
python3 tools/build-water-catalog.py
python3 tools/build-water-catalog.py --check
cargo test -p vearth-gen water_catalog
```

The generator joins relation members into closed rings, preserves holes,
simplifies in local metres within each entry's error budget, and emits static
Rust data. Generation filters entries by cell, then applies the optical mask
only to existing lake/reservoir/inland-water vertices. The small margin and
fade absorb shoreline simplification differences. The catalog creates no wet
geometry, changes no water levels, and adds no runtime texture or render pass.
Protomaps polygon IDs are deliberately not keys: its water postprocessor merges
nearby polygons and can assign different IDs across tiles and zooms.

Each entry's `reference` supports its color character and `temporal` records its
limits. RGB values and extinction are artistic approximations, not measured
optical coefficients. Pink/red entries depict representative bloom states;
Blue Lake depicts its blue summer state. There is no date-driven chemistry,
water-level, salt-crust or freeze/thaw simulation in this catalog.

To add or update a lake, save its complete named OSM way/relation response,
record its version and hash, supply a reference and temporal qualification,
and regenerate. Verify the outline, nearby-water exclusions, and native
captures at both shoreline and aerial distances. Bump `GEN_VERSION` for any
render-affecting palette or geometry-mask change.

## Attribution

The source responses and derived optical-mask geometry are © OpenStreetMap
contributors, available under the [Open Database License 1.0](https://opendatacommons.org/licenses/odbl/1-0/).
See [OpenStreetMap copyright and attribution](https://www.openstreetmap.org/copyright).
The catalog records individual feature URLs. Linked reference photographs and
article text are not bundled. The game's existing OpenStreetMap attribution
continues to apply.
